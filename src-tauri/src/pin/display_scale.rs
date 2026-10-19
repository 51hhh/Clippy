use super::{PinError, PinManager};

/// 读取之前确认原生调用者属于仍存在的 Pin；比例不写回建窗/补偿所用的来源元数据。
pub(super) fn read_for_pin(
    manager: &PinManager,
    caller: &str,
    read: impl FnOnce() -> Result<f64, PinError>,
) -> Result<f64, PinError> {
    if crate::ipc_access::caller_kind(caller) != crate::ipc_access::CallerKind::Pin {
        return Err(PinError::EntryMissing);
    }
    manager.get(caller)?;
    let scale = read()?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(PinError::StateIncomplete);
    }
    Ok(scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pin::model::{PinEntry, PinSource, SharpenSlot};
    use std::cell::Cell;
    use std::sync::Arc;

    fn entry(label: &str) -> PinEntry {
        PinEntry {
            label: label.to_string(),
            source: Arc::new(PinSource::Screenshot {
                png: Arc::new(vec![1, 2, 3]),
            }),
            content_width: 800.0,
            content_height: 600.0,
            scale: 0.75,
            opacity: 0.6,
            locked: true,
            above: true,
            workspace_id: Some(7),
            workspace_group_id: Some(3),
            position: None,
            restore_position: None,
            origin: None,
            #[cfg(target_os = "windows")]
            native_layout: None,
            device_scale: 1.5,
            buffer_scale: 1.5,
            sharpen: Arc::new(SharpenSlot::default()),
        }
    }

    #[test]
    fn windows_pin_live_dpi_read_does_not_rewrite_source_or_presentation() {
        let manager = PinManager::new();
        let original = entry("pin-dpi-1");
        manager.insert(original.clone()).unwrap();
        assert_eq!(
            read_for_pin(&manager, &original.label, || Ok(1.0)).unwrap(),
            1.0
        );
        let after = manager.get(&original.label).unwrap();
        assert!(Arc::ptr_eq(&after.source, &original.source));
        assert!(Arc::ptr_eq(&after.sharpen, &original.sharpen));
        assert_eq!((after.content_width, after.content_height), (800.0, 600.0));
        assert_eq!((after.device_scale, after.buffer_scale), (1.5, 1.5));
        assert_eq!((after.scale, after.opacity), (0.75, 0.6));
        assert!(after.locked && after.above);
        assert_eq!(
            (after.workspace_id, after.workspace_group_id),
            (Some(7), Some(3))
        );
    }

    #[test]
    fn windows_pin_live_dpi_reads_each_current_native_value() {
        let manager = PinManager::new();
        manager.insert(entry("pin-dpi-1")).unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            assert_eq!(
                read_for_pin(&manager, "pin-dpi-1", || Ok(scale)).unwrap(),
                scale
            );
        }
    }

    #[test]
    fn windows_pin_live_dpi_rejects_non_pin_and_unsafe_callers_before_native_read() {
        let manager = PinManager::new();
        for caller in [
            "main",
            "image-viewer-1",
            "pin-workspaces",
            "pin-../other",
            "pin-",
        ] {
            manager.insert(entry(caller)).unwrap();
            let called = Cell::new(false);
            let result = read_for_pin(&manager, caller, || {
                called.set(true);
                Ok(1.0)
            });
            assert!(matches!(result, Err(PinError::EntryMissing)));
            assert!(!called.get());
        }
    }

    #[test]
    fn windows_pin_live_dpi_rejects_closed_pin_before_native_read() {
        let manager = PinManager::new();
        let called = Cell::new(false);
        let result = read_for_pin(&manager, "pin-closed-1", || {
            called.set(true);
            Ok(1.0)
        });
        assert!(matches!(result, Err(PinError::EntryMissing)));
        assert!(!called.get());
    }

    #[test]
    fn windows_pin_live_dpi_rejects_invalid_native_values() {
        let manager = PinManager::new();
        manager.insert(entry("pin-dpi-1")).unwrap();
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(matches!(
                read_for_pin(&manager, "pin-dpi-1", || Ok(scale)),
                Err(PinError::StateIncomplete)
            ));
        }
    }

    #[test]
    fn windows_pin_live_dpi_propagates_native_read_error() {
        let manager = PinManager::new();
        manager.insert(entry("pin-dpi-1")).unwrap();
        let result = read_for_pin(&manager, "pin-dpi-1", || {
            Err(PinError::window("read failed"))
        });
        assert_eq!(result.unwrap_err().to_string(), "read failed");
    }
}
