pub(crate) mod commands;
mod error;
mod manager;
mod model;
mod origins;
mod window;

pub(crate) use commands::create_screenshot_pin;
pub use error::PinError;
pub(crate) use manager::remember_pin_window_position;
pub use manager::PinManager;
pub(crate) use model::PinOrigin;
pub(crate) use origins::{PinFingerprint, PinOriginRegistry};

#[cfg(test)]
mod tests {
    use super::manager::PinManager;
    use super::model::{
        is_safe_pin_label, window_marker, PinEntry, PinOrigin, PinPosition, PinSource,
    };
    use super::window::{
        clamp_pin_position, clamp_span, fit_dimensions, fit_image_content_size, outer_size,
    };
    use std::sync::Arc;
    use tauri::{PhysicalPosition, PhysicalSize};

    fn screenshot_entry(label: &str) -> PinEntry {
        PinEntry {
            label: label.to_string(),
            source: Arc::new(PinSource::Screenshot { png: vec![1, 2, 3] }),
            content_width: 320.0,
            content_height: 180.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            position: None,
            origin: None,
        }
    }

    #[test]
    fn fits_large_images_without_changing_aspect_ratio() {
        assert_eq!(
            fit_dimensions(3840.0, 2160.0, 900.0, 700.0),
            (900.0, 506.25)
        );
    }

    #[test]
    fn sizing_preserves_small_and_extreme_aspect_ratios() {
        assert_eq!(fit_dimensions(120.0, 80.0, 900.0, 700.0), (180.0, 120.0));
        assert_eq!(fit_dimensions(1.0, 1000.0, 900.0, 700.0), (0.7, 700.0));
    }

    /// 没有原始矩形时的内容尺寸：入参是**图片像素**，必须先按屏幕真实缩放折成 CSS 像素。
    /// 漏掉这一步的话，缩放 1.3333 的屏上一张 1052x797 的截图会被当成 1052 CSS 像素显示，
    /// 在屏幕上占 1403 个设备像素——图片被拉大再重采样，也就是"贴出来比原来大一圈还发糊"。
    #[test]
    fn image_pixels_become_css_pixels_through_the_real_display_scale() {
        let scale = 4.0 / 3.0;
        let (width, height) = fit_image_content_size(1052.0, 797.0, scale, 1300.0, 800.0);
        assert!((width - 789.0).abs() < 0.01, "{width}");
        assert!((height - 597.75).abs() < 0.01, "{height}");

        // 缩放 1（X11、整数缩放的屏）必须是恒等变换，不能给这些环境引入新的缩放。
        assert_eq!(
            fit_image_content_size(640.0, 480.0, 1.0, 1300.0, 800.0),
            (640.0, 480.0)
        );
        // 拿不到真实缩放时调用方传 GDK 的数；0 或负数只可能是查询出错，退回 1 而不是除爆。
        assert_eq!(
            fit_image_content_size(640.0, 480.0, 0.0, 1300.0, 800.0),
            (640.0, 480.0)
        );
        // 折算之后仍然超出工作区上限时照旧按比例缩小。
        assert_eq!(
            fit_image_content_size(3840.0, 2160.0, 2.0, 900.0, 700.0),
            (900.0, 506.25)
        );
    }

    #[test]
    fn pin_labels_reject_path_and_query_characters() {
        assert!(is_safe_pin_label("pin-image-123"));
        assert!(!is_safe_pin_label("pin-../../secret"));
        assert!(!is_safe_pin_label("pin-id?x=1"));
    }

    #[test]
    fn outer_size_reserves_controls_and_shadow() {
        assert_eq!(outer_size(400.0, 300.0, 1.0), (468.0, 372.0));
        assert_eq!(outer_size(400.0, 300.0, 0.5), (268.0, 222.0));
    }

    #[test]
    fn manager_tracks_position_and_releases_destroyed_window_state() {
        let manager = PinManager::new();
        manager.insert(screenshot_entry("pin-image-test")).unwrap();

        manager.remember_position("pin-image-test", PhysicalPosition::new(-420, 36));
        assert_eq!(
            manager.get("pin-image-test").unwrap().position,
            Some(PinPosition { x: -420, y: 36 })
        );
        assert_eq!(manager.len(), 1);

        manager.remove_window("pin-image-test");
        assert_eq!(manager.len(), 0);
        assert!(manager.get("pin-image-test").is_err());
    }

    /// `update_pin` 每帧要克隆两份条目（回滚用的旧值 + 更新后的新值）。内容按值放在
    /// `PinEntry` 里的话，滚轮缩放会在主线程上每帧白复制整张 PNG，所以它必须共享。
    #[test]
    fn cloning_an_entry_shares_the_image_bytes_instead_of_copying_them() {
        let manager = PinManager::new();
        manager.insert(screenshot_entry("pin-image-share")).unwrap();

        let first = manager.get("pin-image-share").unwrap();
        let second = manager.get("pin-image-share").unwrap();
        assert!(Arc::ptr_eq(&first.source, &second.source));

        let updated = manager
            .update(
                "pin-image-share",
                &super::model::PinUpdate {
                    scale: Some(2.0),
                    opacity: None,
                    locked: None,
                },
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first.source, &updated.source));
        assert_eq!(updated.scale, 2.0);
    }

    #[test]
    fn manager_rejects_duplicate_labels_without_replacing_entry() {
        let manager = PinManager::new();
        manager.insert(screenshot_entry("pin-image-test")).unwrap();
        assert!(manager.insert(screenshot_entry("pin-image-test")).is_err());
        assert_eq!(manager.len(), 1);
    }

    /// 贴图要"贴回原处"就得让内容区（而不是窗口左上角）压在原始矩形上。
    /// 内容区相对窗口原点偏移 `SHADOW_GUTTER`，所以窗口位置 = 原始矩形 - 12。
    /// 这条换算和 `pin.css` 的 `.pin-media` inset 是一份契约，改一处必须改另一处。
    #[test]
    fn window_origin_offsets_the_content_area_by_the_shadow_gutter() {
        let origin = PinOrigin {
            x: 400.0,
            y: 300.0,
            width: 640.0,
            height: 480.0,
        };
        let (outer_width, outer_height) = outer_size(origin.width, origin.height, 1.0);
        // 内容区宽 = 外框宽 - 左右阴影 - 右侧控件栏
        assert_eq!(outer_width - 12.0 * 2.0 - 44.0, origin.width);
        assert_eq!(outer_height - 12.0 * 2.0 - 48.0, origin.height);
        assert_eq!((origin.x - 12.0, origin.y - 12.0), (388.0, 288.0));
    }

    #[test]
    fn origin_rects_must_be_finite_and_visible() {
        let good = PinOrigin {
            x: -1920.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
        };
        assert_eq!(good.sanitized(), Some(good));
        for bad in [
            PinOrigin {
                x: f64::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            PinOrigin {
                x: 0.0,
                y: f64::INFINITY,
                width: 10.0,
                height: 10.0,
            },
            // 1 像素高的选区做不成窗口，只会把几何算成负数
            PinOrigin {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 1.0,
            },
            PinOrigin {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
        ] {
            assert_eq!(bad.sanitized(), None, "{bad:?} 不该通过校验");
        }
    }

    /// 窗口标题是 GNOME Shell 扩展唯一的查找键，格式必须跟着 label 唯一。
    #[test]
    fn window_marker_is_unique_per_label() {
        assert_eq!(window_marker("pin-image-7"), "Clippy Pin pin-image-7");
        assert_ne!(window_marker("pin-image-7"), window_marker("pin-image-8"));
    }

    #[test]
    fn logical_clamp_keeps_the_window_inside_the_work_area() {
        // 工作区 x∈[0,1000)，窗口宽 300 → 位置上限 700
        assert_eq!(clamp_span(900.0, 0.0, 1000.0, 300.0), 700.0);
        assert_eq!(clamp_span(-50.0, 0.0, 1000.0, 300.0), 0.0);
        assert_eq!(clamp_span(120.0, 0.0, 1000.0, 300.0), 120.0);
        // 窗口比工作区还大时贴边，不能算出比起点更小的值
        assert_eq!(clamp_span(500.0, 24.0, 200.0, 400.0), 24.0);
    }

    #[test]
    fn pin_position_clamps_to_negative_origin_work_area() {
        let work = tauri::PhysicalRect {
            position: PhysicalPosition::new(-1920, 24),
            size: PhysicalSize::new(1920, 1056),
        };
        assert_eq!(
            clamp_pin_position(
                PhysicalPosition::new(-100, 1000),
                PhysicalSize::new(500, 400),
                &work,
            ),
            PhysicalPosition::new(-500, 680)
        );
        assert_eq!(
            clamp_pin_position(
                PhysicalPosition::new(-5000, -5000),
                PhysicalSize::new(4000, 2000),
                &work,
            ),
            work.position
        );
    }
}
