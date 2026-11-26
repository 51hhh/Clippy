use super::{send, App, AppWindow, Message, OcrStatus};
use crate::locale::{message, tr};
use clippy_vision::{health_status, invalidate_executable_cache, NativeOcr};
use slint::Weak;
use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

#[derive(Default)]
pub(in crate::app) struct OcrChecks {
    worker: Option<JoinHandle<()>>,
    next: Option<(Weak<AppWindow>, Target)>,
}

enum Target {
    Disabled,
    Bundled(Arc<NativeOcr>),
    System { manifest: String, bundled: bool },
}

pub(super) fn refresh(app: &mut App, owner: Weak<AppWindow>) {
    let Some(window) = owner.upgrade() else {
        return;
    };
    let target = if !app.config.ocr_enabled {
        Target::Disabled
    } else if let Some(native) = app.vision.native_engine() {
        Target::Bundled(native)
    } else {
        Target::System {
            manifest: app.config.enhanced_ocr_manifest_path.clone(),
            bundled: app.vision.native.is_some(),
        }
    };
    window.set_ocr_status(if matches!(target, Target::Disabled) {
        status("OCR disabled", "")
    } else {
        OcrStatus {
            title: tr("Checking OCR…").into(),
            checking: true,
            ..Default::default()
        }
    });
    app.ocr_checks.next = Some((owner, target));
    app.ocr_checks.start_next();
}

impl OcrChecks {
    fn start_next(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let Some((owner, target)) = self.next.take() else {
            return;
        };
        let Some(window) = owner.upgrade() else {
            return;
        };
        if matches!(target, Target::Disabled) {
            return;
        }
        match thread::Builder::new()
            .name("clippy-ocr-status".into())
            .spawn(move || {
                send(Message::OcrChecked(owner, check(target)));
            }) {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => window.set_ocr_status(status("Could not check OCR", error.to_string())),
        }
    }

    pub(super) fn complete(&mut self, owner: Weak<AppWindow>, status: OcrStatus) {
        self.join();
        if self.next.is_none() {
            if let Some(window) = owner.upgrade() {
                window.set_ocr_status(status);
            }
        }
        self.start_next();
    }

    fn join(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for OcrChecks {
    fn drop(&mut self) {
        self.join();
    }
}

fn check(target: Target) -> OcrStatus {
    match target {
        Target::Disabled => unreachable!("停用状态不启动探测线程"),
        Target::Bundled(native) => match native.check_assets() {
            Ok(()) => status(
                "Bundled PP-OCRv6",
                "Worker and runtime files found. Model checksums match.",
            ),
            Err(error) => status(
                "Bundled OCR needs repair",
                message(
                    "{0}. Reinstall Clippy to restore the bundled files.",
                    &[&error],
                ),
            ),
        },
        Target::System { manifest, bundled } => {
            invalidate_executable_cache();
            let health = health_status(&manifest);
            if health.enhanced_state == "ready" {
                let source = if health.configuration_source == "environment" {
                    "environment"
                } else {
                    "saved manifest"
                };
                return status(
                    "Enhanced OCR",
                    message(
                        "The {0} configuration and its model checksums passed validation.",
                        &[&tr(source)],
                    ),
                );
            }
            let (title, detail) = if health.tesseract_available {
                ("Tesseract", "System Tesseract passed its version check.")
            } else {
                (
                    "OCR unavailable",
                    "Install a Clippy package with OCR or configure system Tesseract.",
                )
            };
            let Some(issue) = health.issue_code else {
                return status(title, detail);
            };
            let reason = match issue {
                "manifest_path_invalid" => "The enhanced manifest path must be absolute.",
                "manifest_missing" => "The enhanced manifest file could not be found.",
                "manifest_format_invalid" => {
                    "The enhanced manifest contains invalid JSON or fields."
                }
                "manifest_contract_invalid" | "parameters_invalid" => {
                    "The enhanced manifest has unsupported options."
                }
                "python_missing" => "The configured Python executable could not be found.",
                "script_missing" | "runtime_module_missing" => {
                    "An enhanced OCR script or module is missing."
                }
                "model_missing" => "A model listed in the enhanced manifest is missing.",
                "model_hash_mismatch" => "An enhanced OCR model failed checksum validation.",
                _ => "The enhanced OCR configuration could not be verified.",
            };
            let action = if health.configuration_source == "environment" {
                " Update or remove CLIPPY_OCR_MANIFEST."
            } else if bundled {
                " Clear the enhanced manifest and save to use bundled OCR."
            } else {
                " Review the enhanced manifest and recheck OCR."
            };
            status(
                title,
                format!("{} {}{}", tr(detail), tr(reason), tr(action)),
            )
        }
    }
}

fn status(title: &str, detail: impl Into<slint::SharedString>) -> OcrStatus {
    OcrStatus {
        title: tr(title).into(),
        detail: tr(detail.into().as_str()).into(),
        checking: false,
    }
}
