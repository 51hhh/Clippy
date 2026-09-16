use super::*;
#[cfg(target_os = "linux")]
use image::{ImageBuffer, Rgba};

#[cfg(target_os = "linux")]
#[test]
fn temporary_screenshot_guard_cleans_original_and_replaced_files() {
    use super::backends::TemporaryScreenshotFile;

    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.png");
    let replacement = directory.path().join("replacement.png");
    std::fs::write(&original, b"first").unwrap();
    std::fs::write(&replacement, b"second").unwrap();

    {
        let mut screenshot = TemporaryScreenshotFile::new(original.clone());
        screenshot.replace_path(replacement.clone());
        assert!(!original.exists());
    }

    assert!(!replacement.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn gnome_shell_screenshot_path_must_stay_in_private_directory() {
    use super::backends::validate_gnome_shell_screenshot_path;

    let directory = std::path::Path::new("/tmp/clippy-private");
    assert!(validate_gnome_shell_screenshot_path(directory, &directory.join("shot.png")).is_ok());
    assert!(validate_gnome_shell_screenshot_path(
        directory,
        std::path::Path::new("/tmp/outside.png")
    )
    .is_err());
}

/// 竖屏的像素方向。**方向搞反在横屏上永远看不出来**，所以这里逐像素钉住：
/// 一张左上角唯一有色的图，转 90 度之后那个像素必须落到右上角。
#[cfg(target_os = "linux")]
#[test]
fn a_quarter_turn_moves_the_top_left_pixel_to_the_top_right() {
    use libwayshot_xcap::reexport::Transform;

    // 4x2 的横向图，只有 (0,0) 是红的。转 90° 后变成 2x4，红点应当在 (1,0)。
    let mut image = ImageBuffer::from_pixel(4, 2, Rgba([0, 0, 0, 255]));
    image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));

    let rotated = super::backends::apply_output_transform(image.clone(), Transform::_90);
    assert_eq!((rotated.width(), rotated.height()), (2, 4));
    assert_eq!(*rotated.get_pixel(1, 0), Rgba([255, 0, 0, 255]));

    // 270° 是另一头：红点落到左下角。
    let rotated = super::backends::apply_output_transform(image.clone(), Transform::_270);
    assert_eq!((rotated.width(), rotated.height()), (2, 4));
    assert_eq!(*rotated.get_pixel(0, 3), Rgba([255, 0, 0, 255]));

    // 镜像只翻不转，尺寸不变，红点跑到右上角。
    let flipped = super::backends::apply_output_transform(image.clone(), Transform::Flipped);
    assert_eq!((flipped.width(), flipped.height()), (4, 2));
    assert_eq!(*flipped.get_pixel(3, 0), Rgba([255, 0, 0, 255]));

    // Normal 必须是恒等：绝大多数用户走的是这一条，多一次拷贝都不该有语义变化。
    let untouched = super::backends::apply_output_transform(image.clone(), Transform::Normal);
    assert_eq!((untouched.width(), untouched.height()), (4, 2));
    assert_eq!(*untouched.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
}

/// 八个 transform 里只有 90/270（含镜像版本）换宽高。判错任意一个，那块屏的缩放
/// 就会变成 1.7778 这种虚构值，帧尺寸和坐标全跟着错。
#[cfg(target_os = "linux")]
#[test]
fn only_the_quarter_turns_swap_the_axes() {
    use libwayshot_xcap::reexport::Transform;

    for transform in [
        Transform::_90,
        Transform::_270,
        Transform::Flipped90,
        Transform::Flipped270,
    ] {
        assert!(
            super::backends::transform_swaps_axes(transform),
            "{transform:?}"
        );
    }
    // 镜像只翻不转，对宽高没有影响。
    for transform in [
        Transform::Normal,
        Transform::_180,
        Transform::Flipped,
        Transform::Flipped180,
    ] {
        assert!(
            !super::backends::transform_swaps_axes(transform),
            "{transform:?}"
        );
    }
}

#[test]
fn portal_file_uri_decodes_to_local_path() {
    let path = portal_screenshot_uri_to_path("file:///tmp/Clippy%20Shot.png").unwrap();
    assert_eq!(path, std::path::PathBuf::from("/tmp/Clippy Shot.png"));
}

#[test]
fn encode_png_compresses_simple_screenshot_data() {
    let width = 64;
    let height = 64;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for _ in 0..(width * height) {
        rgba.extend_from_slice(&[240, 240, 240, 255]);
    }

    let png = encode_png(&rgba, width as u32, height as u32).unwrap();

    assert!(png.len() < rgba.len() / 4);
    assert_eq!(png_dimensions(&png).unwrap(), (width as u32, height as u32));
}

/// `png_dimensions` 只读头，`validate_png` 解整张——这条测试钉住的就是这个分工。
/// 截断的 PNG 头是完好的：前者照样报出尺寸，后者必须失败，否则信任边界那道校验白写了。
#[test]
fn reading_the_header_and_decoding_the_whole_image_are_different_promises() {
    let rgba: Vec<u8> = (0..(32 * 16))
        .flat_map(|index| [(index % 256) as u8, 40, 200, 255])
        .collect();
    let png = encode_png(&rgba, 32, 16).unwrap();
    let truncated = &png[..png.len() * 2 / 3];

    assert_eq!(png_dimensions(&png).unwrap(), (32, 16));
    assert_eq!(validate_png(&png).unwrap(), (32, 16));
    assert_eq!(
        png_dimensions(truncated).unwrap(),
        (32, 16),
        "IHDR 还在，只读头就该读得出来"
    );
    assert!(
        validate_png(truncated).is_err(),
        "整张解码必须认出图像数据被截断"
    );
}

/// 完全不是 PNG 的字节，只读头也必须拒绝：`save_png` 就靠它挡住误传的载荷。
#[test]
fn header_reading_still_rejects_bytes_that_are_not_png_at_all() {
    assert!(png_dimensions(b"this is not a png at all").is_err());
    assert!(png_dimensions(&[]).is_err());
}

/// 原始像素这条路的三条规矩：stride 正好一行时**原样交出去**（8 Mpx 重排一次要十几毫秒，
/// 而这条路存在的理由就是省时间）；stride 更大时按行取前 `width * 4` 字节重排，
/// 不能把行内填充当成像素（否则整张图会一行一行地斜）；字节不够就报错，不能读出边界。
#[cfg(target_os = "linux")]
#[test]
fn raw_area_tiles_are_taken_as_is_or_repacked_by_row() {
    use crate::capture::AreaCapture;

    let directory = tempfile::tempdir().unwrap();

    // 2x2 像素，stride 正好是一行。
    let tight = directory.path().join("tight.rgba");
    let pixels: Vec<u8> = (0..16).collect();
    std::fs::write(&tight, &pixels).unwrap();
    let (width, height, rgba) = load_area_tile(&AreaCapture::Raw {
        path: tight,
        width: 2,
        height: 2,
        stride: 8,
    })
    .expect("stride 等于一行时应当直接读出来");
    assert_eq!((width, height), (2, 2));
    assert_eq!(&rgba[..], &pixels[..]);

    // 同样 2x2，但每行末尾多 4 字节填充，重排后必须只剩像素。
    let padded = directory.path().join("padded.rgba");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&pixels[0..8]);
    bytes.extend_from_slice(&[0xFF; 4]);
    bytes.extend_from_slice(&pixels[8..16]);
    bytes.extend_from_slice(&[0xFF; 4]);
    std::fs::write(&padded, &bytes).unwrap();
    let (_, _, rgba) = load_area_tile(&AreaCapture::Raw {
        path: padded,
        width: 2,
        height: 2,
        stride: 12,
    })
    .expect("有行内填充时应当重排");
    assert_eq!(&rgba[..], &pixels[..]);

    // 文件比声明的尺寸小：宁可整体退回整屏那条路，也不要拿半张画面铺覆盖层。
    let truncated = directory.path().join("truncated.rgba");
    std::fs::write(&truncated, &pixels[..12]).unwrap();
    assert!(load_area_tile(&AreaCapture::Raw {
        path: truncated,
        width: 2,
        height: 2,
        stride: 8,
    })
    .is_err());
}

/// 逐屏取画面对镜像屏只发一次请求：两块屏共用同一个逻辑矩形**且缩放相同**时取出来的
/// 像素一模一样，多发一次就是让用户多等一次读回。同时保证对照表把两块屏都指回那一份。
///
/// 反过来，缩放不同就**不能**并——缩放决定像素尺寸，同一个矩形按 1.3333 和 1.0 取出来
/// 根本不是同一张画面，并了会让其中一块屏拿到别人分辨率的帧。
#[cfg(target_os = "linux")]
#[test]
fn mirrored_monitors_share_a_single_area_screenshot() {
    let rect = |x: i32, width: u32| Rect {
        x,
        y: 0,
        width,
        height: 1200,
    };
    let area = |x: i32, width: u32, scale: f64| crate::capture::CaptureArea {
        x,
        y: 0,
        width,
        height: 1200,
        scale,
    };
    let monitors = vec![
        MonitorInfo {
            id: 1,
            rect: rect(0, 1920),
            scale_factor: 1.3333334,
        },
        // 投影：和 #1 完全同一个逻辑矩形、同一个缩放。
        MonitorInfo {
            id: 2,
            rect: rect(0, 1920),
            scale_factor: 1.3333334,
        },
        MonitorInfo {
            id: 3,
            rect: rect(1920, 2560),
            scale_factor: 1.5,
        },
    ];

    let (areas, assignment) = dedupe_monitor_areas(&monitors);
    assert_eq!(
        areas,
        vec![
            area(0, 1920, f64::from(1.3333334_f32)),
            area(1920, 2560, 1.5)
        ]
    );
    assert_eq!(assignment, vec![0, 0, 1]);

    // 同矩形不同缩放：两次请求，各拿自己分辨率的画面。
    let mixed = vec![
        MonitorInfo {
            id: 1,
            rect: rect(0, 1920),
            scale_factor: 1.3333334,
        },
        MonitorInfo {
            id: 2,
            rect: rect(0, 1920),
            scale_factor: 1.0,
        },
    ];
    let (areas, assignment) = dedupe_monitor_areas(&mixed);
    assert_eq!(areas.len(), 2);
    assert_eq!(assignment, vec![0, 1]);
}

#[test]
fn portal_mapping_splits_horizontal_monitors() {
    let monitors = vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: 1.0,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 100,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: 1.0,
        },
    ];
    let desktop = monitor_union(&monitors).unwrap();

    assert_eq!(
        scaled_monitor_rect(&monitors[0].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
        ImageRect {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        }
    );
    assert_eq!(
        scaled_monitor_rect(&monitors[1].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
        ImageRect {
            x: 200,
            y: 0,
            width: 200,
            height: 100,
        }
    );
}

#[test]
fn compose_places_left_1x_right_2x_monitors_without_gap() {
    let monitors = horizontal_monitors(1.0, 2.0);
    let frames = vec![
        solid_frame(1, 100, 50, 1.0, [255, 0, 0, 255]),
        solid_frame(2, 200, 100, 2.0, [0, 0, 255, 255]),
    ];

    let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

    assert_eq!((width, height), (300, 100));
    assert_eq!(pixel_at(&rgba, width, 99, 25), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&rgba, width, 100, 25), [0, 0, 255, 255]);
    assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
}

#[test]
fn compose_places_left_2x_right_1x_monitors_without_overlap() {
    let monitors = horizontal_monitors(2.0, 1.0);
    let frames = vec![
        solid_frame(1, 200, 100, 2.0, [255, 0, 0, 255]),
        solid_frame(2, 100, 50, 1.0, [0, 0, 255, 255]),
    ];

    let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

    assert_eq!((width, height), (300, 100));
    assert_eq!(pixel_at(&rgba, width, 199, 25), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&rgba, width, 200, 25), [0, 0, 255, 255]);
    assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
}

/// 切舞台图这条路的**像素**部分：每块屏必须从图里正确的位置取像素。
///
/// 几何那半边不在这里测——它由 `tests/fixtures/monitor-layouts/*.json` 逐个环境覆盖
/// （包括混合缩放、镜像、旋转、负坐标、陈几何），驱动的是同一个 `plan_stage_split`。
/// 那样一条新环境的成本是一个 json 文件，而且不必为了走完整条路去凑一张几十兆的假图
/// （6720x2412 的 RGBA 是 64 MB）。这里只留一条最小的，钉住"计划出来的裁剪真的被用上了"。
#[cfg(target_os = "linux")]
#[test]
fn split_portal_screenshot_crops_each_monitor_from_the_right_place() {
    let monitors = horizontal_monitors(1.0, 1.0);
    // 左半红、右半蓝的舞台图。切完之后左屏应当全红、右屏全蓝。
    let stage = ImageBuffer::from_fn(200, 50, |x, _| {
        if x < 100 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });

    let (adjusted, frames) = split_portal_screenshot(monitors.clone(), stage).unwrap();

    assert_eq!(adjusted[0].rect, monitors[0].rect);
    assert_eq!(adjusted[1].rect, monitors[1].rect);
    assert_eq!((frames[0].width, frames[0].height), (100, 50));
    assert_eq!(pixel_at(&frames[0].rgba, 100, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&frames[0].rgba, 100, 99, 49), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&frames[1].rgba, 100, 0, 0), [0, 0, 255, 255]);
    assert_eq!(pixel_at(&frames[1].rgba, 100, 99, 49), [0, 0, 255, 255]);
}

/// 镜像屏（投影）的两块屏共用同一个矩形。像素**必须是同一份**：抠第二遍等于在
/// 截图这条路上白花一次全屏拷贝加一份同样大小的内存，1080p 就是 8 MB。
#[cfg(target_os = "linux")]
#[test]
fn mirrored_monitors_share_one_pixel_buffer() {
    let mirrored = vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 200,
                height: 50,
            },
            scale_factor: 1.0,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 0,
                y: 0,
                width: 200,
                height: 50,
            },
            scale_factor: 1.0,
        },
    ];
    let stage = ImageBuffer::from_fn(200, 50, |x, _| {
        if x < 100 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });

    let (adjusted, frames) = split_portal_screenshot(mirrored, stage).unwrap();

    // 两块屏都还在——镜像不是"少一块屏"，覆盖层和窗口候选照旧按两块屏走。
    assert_eq!(adjusted.len(), 2);
    assert_eq!(frames.len(), 2);
    assert_eq!((frames[1].width, frames[1].height), (200, 50));
    assert!(
        std::sync::Arc::ptr_eq(&frames[0].rgba, &frames[1].rgba),
        "镜像屏的像素被抠了两遍"
    );
}

/// 除数用错的直接后果，单独钉住：同一个矩形换个除数就被改写成 1.125 倍。
#[test]
fn normalize_geometry_depends_on_which_scale_the_pixel_width_belongs_to() {
    let rect = Rect {
        x: 2560,
        y: 408,
        width: 1920,
        height: 1200,
    };
    // 舞台图裁剪宽度属于"桌面最大缩放"坐标系，用它当除数是恒等变换。
    assert_eq!(normalize_monitor_geometry(rect, 1.5, 2880), rect);
    // 换成这块屏自己的缩放就会算出偏大的逻辑尺寸，这正是多屏错位的来源。
    assert_eq!(
        normalize_monitor_geometry(rect, 1.333_333_4, 2880),
        Rect {
            x: 2880,
            y: 459,
            width: 2160,
            height: 1350,
        }
    );
}

fn horizontal_monitors(left_scale: f32, right_scale: f32) -> Vec<MonitorInfo> {
    vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: left_scale,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 100,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: right_scale,
        },
    ]
}

fn solid_frame(
    monitor_id: u32,
    width: u32,
    height: u32,
    scale_factor: f32,
    color: [u8; 4],
) -> FrozenFrame {
    FrozenFrame {
        monitor_id,
        rgba: Arc::from(solid_rgba(width, height, color)),
        width,
        height,
        scale_factor,
    }
}

fn solid_rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..width * height {
        rgba.extend_from_slice(&color);
    }
    rgba
}

fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * width + x) * 4) as usize;
    [
        rgba[start],
        rgba[start + 1],
        rgba[start + 2],
        rgba[start + 3],
    ]
}
