//! 进程级更新状态：窗口只观察，下载/安装任务不随 WebView 销毁而丢失。
use crate::platform::InstallType;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Idle,
    Available,
    Installing,
    Installed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateSnapshot {
    pub revision: u64,
    pub status: UpdateStatus,
    pub version: Option<String>,
    pub body: String,
    pub install_type: InstallType,
    pub downloaded: u64,
    pub total: Option<u64>,
}

struct UpdateMachine<T> {
    snapshot: UpdateSnapshot,
    update: Option<T>,
    operation: u64,
}

impl<T: Clone> UpdateMachine<T> {
    fn can_restart(&self) -> bool {
        self.snapshot.status == UpdateStatus::Installed
            && self.snapshot.install_type != InstallType::Windows
    }
    fn new(install_type: InstallType) -> Self {
        Self {
            snapshot: UpdateSnapshot {
                revision: 0,
                status: UpdateStatus::Idle,
                version: None,
                body: String::new(),
                install_type,
                downloaded: 0,
                total: None,
            },
            update: None,
            operation: 0,
        }
    }

    fn checked(&mut self, revision: u64, update: Option<(T, String, String)>) {
        // 检查期间可能已由另一窗口开始安装，旧检查不能覆盖其进度/终态。
        if self.snapshot.revision != revision {
            return;
        }
        self.snapshot.revision += 1;
        self.snapshot.downloaded = 0;
        self.snapshot.total = None;
        match update {
            Some((update, version, body)) => {
                self.update = Some(update);
                self.snapshot.version = Some(version);
                self.snapshot.body = body;
                self.snapshot.status = UpdateStatus::Available;
            }
            None => {
                self.update = None;
                self.snapshot.version = None;
                self.snapshot.body.clear();
                self.snapshot.status = UpdateStatus::Idle;
            }
        }
    }

    fn begin_install(&mut self, version: &str) -> Result<Option<(u64, T)>, String> {
        if matches!(
            self.snapshot.status,
            UpdateStatus::Installing | UpdateStatus::Installed
        ) {
            return Ok(None);
        }
        if self.snapshot.version.as_deref() != Some(version) {
            return Err("更新版本已变化，请重新检查".into());
        }
        if !matches!(
            self.snapshot.install_type,
            InstallType::Appimage | InstallType::Macos | InstallType::Windows
        ) {
            return Err("当前安装类型需要手动更新".into());
        }
        let update = self.update.clone().ok_or("没有可安装的更新")?;
        self.operation += 1;
        self.snapshot.revision += 1;
        self.snapshot.status = UpdateStatus::Installing;
        self.snapshot.downloaded = 0;
        self.snapshot.total = None;
        Ok(Some((self.operation, update)))
    }

    fn progress(&mut self, operation: u64, chunk: usize, total: Option<u64>) {
        if self.operation != operation || self.snapshot.status != UpdateStatus::Installing {
            return;
        }
        self.snapshot.revision += 1;
        self.snapshot.downloaded = self.snapshot.downloaded.saturating_add(chunk as u64);
        self.snapshot.total = total;
    }

    fn finish(&mut self, operation: u64, succeeded: bool) {
        if self.operation != operation || self.snapshot.status != UpdateStatus::Installing {
            return;
        }
        self.snapshot.revision += 1;
        self.snapshot.status = if succeeded {
            UpdateStatus::Installed
        } else {
            UpdateStatus::Failed
        };
        if succeeded {
            self.update = None;
        }
    }
}

pub struct AppUpdater {
    machine: Mutex<UpdateMachine<Update>>,
    check_gate: tokio::sync::Mutex<()>,
}

impl Default for AppUpdater {
    fn default() -> Self {
        Self::new()
    }
}

impl AppUpdater {
    pub fn new() -> Self {
        Self {
            machine: Mutex::new(UpdateMachine::new(crate::platform::current_install_type())),
            check_gate: tokio::sync::Mutex::new(()),
        }
    }

    fn snapshot(&self) -> Result<UpdateSnapshot, String> {
        Ok(self
            .machine
            .lock()
            .map_err(|error| error.to_string())?
            .snapshot
            .clone())
    }

    pub fn ensure_restart_allowed(&self) -> Result<(), String> {
        if self
            .machine
            .lock()
            .map_err(|error| error.to_string())?
            .can_restart()
        {
            Ok(())
        } else {
            Err("更新尚未安装完成，不能通过更新入口重启".into())
        }
    }

    fn publish(&self, app: &tauri::AppHandle) {
        match self.snapshot().and_then(|snapshot| {
            app.emit("app-update-state", snapshot)
                .map_err(|e| e.to_string())
        }) {
            Ok(()) => {}
            Err(error) => log::warn!("更新状态通知失败: {error}"),
        }
    }
}

#[tauri::command]
pub fn get_app_update_state(app: tauri::AppHandle) -> Result<UpdateSnapshot, String> {
    app.state::<Arc<AppUpdater>>().snapshot()
}

#[tauri::command]
pub async fn check_app_update(app: tauri::AppHandle) -> Result<UpdateSnapshot, String> {
    let coordinator = app.state::<Arc<AppUpdater>>().inner().clone();
    let requested_revision = coordinator.snapshot()?.revision;
    let _check = coordinator.check_gate.lock().await;
    let before = coordinator.snapshot()?;
    // 同时到来的检查共享首个结果；后续手动检查仍可发现较 Available 更新的版本。
    if before.revision != requested_revision
        || matches!(
            before.status,
            UpdateStatus::Installing | UpdateStatus::Installed
        )
    {
        return Ok(before);
    }
    let result = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;
    coordinator
        .machine
        .lock()
        .map_err(|error| error.to_string())?
        .checked(
            before.revision,
            result.map(|update| {
                let version = update.version.clone();
                let body = update.body.clone().unwrap_or_default();
                (update, version, body)
            }),
        );
    coordinator.publish(&app);
    coordinator.snapshot()
}

#[tauri::command]
pub fn install_app_update(
    version: String,
    app: tauri::AppHandle,
) -> Result<UpdateSnapshot, String> {
    let coordinator = app.state::<Arc<AppUpdater>>().inner().clone();
    let work = coordinator
        .machine
        .lock()
        .map_err(|error| error.to_string())?
        .begin_install(&version)?;
    if let Some((operation, update)) = work {
        coordinator.publish(&app);
        let task_coordinator = coordinator.clone();
        // 任务只持有 AppHandle，不持有发起 WebView。窗口关闭不取消已接纳的更新。
        tauri::async_runtime::spawn(async move {
            let result = async {
                let bytes = update
                    .download(
                        |chunk, total| {
                            match task_coordinator.machine.lock() {
                                Ok(mut machine) => machine.progress(operation, chunk, total),
                                Err(error) => log::warn!("更新进度状态不可用: {error}"),
                            }
                            task_coordinator.publish(&app);
                        },
                        || {},
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                // 解包/文件替换离开 UI 线程；Windows 安装器由插件处理退出，不重启旧 exe。
                tauri::async_runtime::spawn_blocking(move || {
                    update.install(bytes).map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| error.to_string())?
            }
            .await;
            if let Err(error) = &result {
                log::warn!("应用更新失败: {error}");
            }
            match task_coordinator.machine.lock() {
                Ok(mut machine) => machine.finish(operation, result.is_ok()),
                Err(error) => log::error!("更新终态无法记录: {error}"),
            }
            task_coordinator.publish(&app);
        });
    }
    coordinator.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn available() -> UpdateMachine<&'static str> {
        let mut machine = UpdateMachine::new(InstallType::Appimage);
        machine.checked(0, Some(("fixture", "2.0.0".into(), "notes".into())));
        machine
    }

    #[test]
    fn concurrent_windows_claim_only_one_native_install() {
        let state = Arc::new(Mutex::new(available()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let state = state.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    state
                        .lock()
                        .unwrap()
                        .begin_install("2.0.0")
                        .unwrap()
                        .is_some()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn reopened_windows_observe_progress_and_installed_without_reinstall() {
        let state = Arc::new(Mutex::new(available()));
        let window = state.clone();
        let (operation, _) = window
            .lock()
            .unwrap()
            .begin_install("2.0.0")
            .unwrap()
            .unwrap();
        drop(window);
        let mut reopened = state.lock().unwrap();
        reopened.progress(operation, 50, Some(100));
        assert_eq!(reopened.snapshot.downloaded, 50);
        assert!(reopened.begin_install("2.0.0").unwrap().is_none());
        reopened.finish(operation, true);
        assert_eq!(reopened.snapshot.status, UpdateStatus::Installed);
        assert!(reopened.can_restart());
        assert!(reopened.begin_install("2.0.0").unwrap().is_none());
    }

    #[test]
    fn failed_install_can_retry_but_stale_progress_and_check_cannot_overwrite_it() {
        let mut machine = available();
        let checked_revision = machine.snapshot.revision;
        let (first, _) = machine.begin_install("2.0.0").unwrap().unwrap();
        machine.checked(checked_revision, None);
        assert_eq!(machine.snapshot.status, UpdateStatus::Installing);
        machine.finish(first, false);
        let (second, _) = machine.begin_install("2.0.0").unwrap().unwrap();
        machine.progress(first, 99, Some(100));
        machine.finish(first, true);
        assert_eq!(machine.snapshot.downloaded, 0);
        assert_eq!(machine.snapshot.status, UpdateStatus::Installing);
        machine.finish(second, true);
        machine.progress(second, 99, Some(100));
        assert_eq!(machine.snapshot.status, UpdateStatus::Installed);
        assert_eq!(machine.snapshot.downloaded, 0);
    }

    #[test]
    fn stale_version_and_manual_install_types_cannot_start() {
        let mut machine = available();
        assert!(machine.begin_install("1.0.0").is_err());
        machine.snapshot.install_type = InstallType::Deb;
        assert!(machine.begin_install("2.0.0").is_err());
        assert_eq!(machine.snapshot.status, UpdateStatus::Available);
        assert!(!machine.can_restart());
        machine.snapshot.install_type = InstallType::Windows;
        machine.snapshot.status = UpdateStatus::Installed;
        assert!(!machine.can_restart());
    }

    #[test]
    fn later_check_can_replace_available_but_cannot_replace_started_install() {
        let mut machine = available();
        machine.checked(
            machine.snapshot.revision,
            Some(("next fixture", "3.0.0".into(), "next notes".into())),
        );
        assert_eq!(machine.snapshot.version.as_deref(), Some("3.0.0"));
        let check_revision = machine.snapshot.revision;
        machine.begin_install("3.0.0").unwrap().unwrap();
        machine.checked(check_revision, None);
        assert_eq!(machine.snapshot.status, UpdateStatus::Installing);
        assert_eq!(machine.snapshot.version.as_deref(), Some("3.0.0"));
    }
}
