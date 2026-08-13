use super::model::{is_safe_pin_label, PinEntry, PinPosition, PinUpdate};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::PhysicalPosition;

#[derive(Default)]
pub struct PinManager {
    entries: Mutex<HashMap<String, PinEntry>>,
}

impl PinManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert(&self, entry: PinEntry) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .insert(entry.label.clone(), entry);
        Ok(())
    }

    pub(super) fn get(&self, label: &str) -> Result<PinEntry, String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .get(label)
            .cloned()
            .ok_or_else(|| "贴图不存在或已经关闭".to_string())
    }

    pub(super) fn remove(&self, label: &str) -> Result<Option<PinEntry>, String> {
        Ok(self
            .entries
            .lock()
            .map_err(|error| error.to_string())?
            .remove(label))
    }

    pub(super) fn update(&self, label: &str, update: &PinUpdate) -> Result<PinEntry, String> {
        let mut entries = self.entries.lock().map_err(|error| error.to_string())?;
        let entry = entries
            .get_mut(label)
            .ok_or_else(|| "贴图不存在或已经关闭".to_string())?;
        if let Some(scale) = update.scale {
            entry.scale = scale.clamp(0.25, 4.0);
        }
        if let Some(opacity) = update.opacity {
            entry.opacity = opacity.clamp(0.15, 1.0);
        }
        if let Some(locked) = update.locked {
            entry.locked = locked;
        }
        Ok(entry.clone())
    }

    pub(super) fn remember_position(&self, label: &str, position: PhysicalPosition<i32>) {
        if !is_safe_pin_label(label) {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if let Some(entry) = entries.get_mut(label) {
            entry.position = Some(PinPosition {
                x: position.x,
                y: position.y,
            });
        }
    }

    pub fn remove_window(&self, label: &str) {
        if is_safe_pin_label(label) {
            let _ = self.remove(label);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }
}

pub(crate) fn remember_pin_window_position(
    manager: &PinManager,
    label: &str,
    position: PhysicalPosition<i32>,
) {
    manager.remember_position(label, position);
}
