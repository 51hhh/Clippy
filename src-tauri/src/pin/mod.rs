pub(crate) mod commands;
mod error;
mod manager;
mod model;
mod window;

pub(crate) use commands::create_screenshot_pin;
pub use error::PinError;
pub(crate) use manager::remember_pin_window_position;
pub use manager::PinManager;

#[cfg(test)]
mod tests {
    use super::manager::PinManager;
    use super::model::{is_safe_pin_label, PinEntry, PinPosition, PinSource};
    use super::window::{clamp_pin_position, fit_dimensions, outer_size};
    use tauri::{PhysicalPosition, PhysicalSize};

    fn screenshot_entry(label: &str) -> PinEntry {
        PinEntry {
            label: label.to_string(),
            source: PinSource::Screenshot { png: vec![1, 2, 3] },
            content_width: 320.0,
            content_height: 180.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            position: None,
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

    #[test]
    fn manager_rejects_duplicate_labels_without_replacing_entry() {
        let manager = PinManager::new();
        manager.insert(screenshot_entry("pin-image-test")).unwrap();
        assert!(manager.insert(screenshot_entry("pin-image-test")).is_err());
        assert_eq!(manager.len(), 1);
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
