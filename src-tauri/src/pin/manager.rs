use super::error::PinError;
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

    pub(super) fn insert(&self, entry: PinEntry) -> Result<(), PinError> {
        let mut entries = self.entries.lock().map_err(PinError::state_lock)?;
        if entries.contains_key(&entry.label) {
            return Err(PinError::AlreadyExists);
        }
        entries.insert(entry.label.clone(), entry);
        Ok(())
    }

    pub(super) fn get(&self, label: &str) -> Result<PinEntry, PinError> {
        self.entries
            .lock()
            .map_err(PinError::state_lock)?
            .get(label)
            .cloned()
            .ok_or(PinError::EntryMissing)
    }

    pub(super) fn replace(&self, entry: PinEntry) -> Result<(), PinError> {
        self.entries
            .lock()
            .map_err(PinError::state_lock)?
            .insert(entry.label.clone(), entry);
        Ok(())
    }

    pub(super) fn remove(&self, label: &str) -> Result<Option<PinEntry>, PinError> {
        // 窗口没了，还在后台等它出现的摆放重试也该停下来。
        super::window::forget_placement(label);
        Ok(self
            .entries
            .lock()
            .map_err(PinError::state_lock)?
            .remove(label))
    }

    pub(super) fn update(&self, label: &str, update: &PinUpdate) -> Result<PinEntry, PinError> {
        let mut entries = self.entries.lock().map_err(PinError::state_lock)?;
        let entry = entries.get_mut(label).ok_or(PinError::EntryMissing)?;
        if let Some(scale) = update.scale {
            entry.scale = scale.clamp(0.25, 4.0);
        }
        if let Some(opacity) = update.opacity {
            entry.opacity = opacity.clamp(0.15, 1.0);
        }
        if let Some(locked) = update.locked {
            entry.locked = locked;
        }
        if let Some(above) = update.above {
            entry.above = above;
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

    /// 当前开着图钉的那些贴图。截图期间要让它们暂时退出置顶层
    /// （见 `super::commands::lower_pins_for_capture`）。
    pub(super) fn labels_above(&self) -> Vec<String> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries
            .values()
            .filter(|entry| entry.above)
            .map(|entry| entry.label.clone())
            .collect()
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
