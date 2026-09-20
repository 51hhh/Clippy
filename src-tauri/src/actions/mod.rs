//! PX-ACT-01 类型化内置动作核心。
//!
//! 该模块建立静态注册表、参数边界、窗口角色权限和请求代次，并分阶段接入复用既有业务服务的
//! 领域适配器；受限 IPC 与启动器 UI 尚未开放。动作参数不接受路径、URL、像素或可执行命令。

mod adapters;

use crate::ipc_access::CallerKind;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_SOURCE_ID_BYTES: usize = 128;
const MAX_REQUEST_SLOT_BYTES: usize = 96;
const ALL_PLATFORMS: &[ActionPlatform] = &[
    ActionPlatform::Linux,
    ActionPlatform::Windows,
    ActionPlatform::Macos,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionValueKind {
    Unit,
    OwnedImage,
    Text,
    TranslationRequest,
    CaptureSession,
    RecognizedText,
    DetectedCodes,
    TranslatedText,
    SavedPath,
    WindowHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum ActionPermission {
    #[serde(rename = "screen.capture")]
    ScreenCapture,
    #[serde(rename = "image.local_analysis")]
    LocalImageAnalysis,
    #[serde(rename = "translation.network")]
    NetworkTranslation,
    #[serde(rename = "clipboard.write")]
    ClipboardWrite,
    #[serde(rename = "file.write")]
    FileWrite,
    #[serde(rename = "window.create")]
    WindowCreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ActionPlatform {
    Linux,
    Windows,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActionDescriptor {
    pub id: &'static str,
    pub input: ActionValueKind,
    pub output: ActionValueKind,
    pub permissions: &'static [ActionPermission],
    pub cancellable: bool,
    pub platforms: &'static [ActionPlatform],
}

const ACTIONS: &[ActionDescriptor] = &[
    ActionDescriptor {
        id: "capture.start",
        input: ActionValueKind::Unit,
        output: ActionValueKind::CaptureSession,
        permissions: &[ActionPermission::ScreenCapture],
        cancellable: false,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "image.ocr",
        input: ActionValueKind::OwnedImage,
        output: ActionValueKind::RecognizedText,
        permissions: &[ActionPermission::LocalImageAnalysis],
        cancellable: true,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "image.pin",
        input: ActionValueKind::OwnedImage,
        output: ActionValueKind::WindowHandle,
        permissions: &[ActionPermission::WindowCreate],
        cancellable: false,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "image.save",
        input: ActionValueKind::OwnedImage,
        output: ActionValueKind::SavedPath,
        permissions: &[ActionPermission::FileWrite],
        cancellable: false,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "image.scan_codes",
        input: ActionValueKind::OwnedImage,
        output: ActionValueKind::DetectedCodes,
        permissions: &[ActionPermission::LocalImageAnalysis],
        cancellable: true,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "text.copy",
        input: ActionValueKind::Text,
        output: ActionValueKind::Unit,
        permissions: &[ActionPermission::ClipboardWrite],
        cancellable: false,
        platforms: ALL_PLATFORMS,
    },
    ActionDescriptor {
        id: "text.translate",
        input: ActionValueKind::TranslationRequest,
        output: ActionValueKind::TranslatedText,
        permissions: &[ActionPermission::NetworkTranslation],
        cancellable: true,
        platforms: ALL_PLATFORMS,
    },
];

pub(super) fn descriptors() -> &'static [ActionDescriptor] {
    ACTIONS
}

pub(super) enum ActionInput {
    Unit,
    OwnedImage {
        source_id: String,
        source_version: u64,
    },
    Text(String),
    Translation {
        text: String,
        source_language: Option<String>,
        target_language: String,
    },
}

fn descriptor(id: &str) -> Option<&'static ActionDescriptor> {
    ACTIONS.iter().find(|descriptor| descriptor.id == id)
}

fn action_allowed(caller: CallerKind, action_id: &str) -> bool {
    match caller {
        CallerKind::Main | CallerKind::Launcher => true,
        CallerKind::CaptureOverlay | CallerKind::ImageViewer => matches!(
            action_id,
            "image.ocr"
                | "image.pin"
                | "image.save"
                | "image.scan_codes"
                | "text.copy"
                | "text.translate"
        ),
        CallerKind::Pin => matches!(action_id, "image.save" | "text.copy"),
        CallerKind::Settings
        | CallerKind::LongshotController
        | CallerKind::RecordingControl
        | CallerKind::Unknown => false,
    }
}

fn validate_input(kind: ActionValueKind, input: &Value) -> Result<ActionInput, ActionError> {
    match kind {
        ActionValueKind::Unit => exact_object(input, &[]).map(|_| ActionInput::Unit),
        ActionValueKind::OwnedImage => validate_owned_image(input),
        ActionValueKind::Text => validate_text(input),
        ActionValueKind::TranslationRequest => validate_translation_request(input),
        ActionValueKind::CaptureSession
        | ActionValueKind::RecognizedText
        | ActionValueKind::DetectedCodes
        | ActionValueKind::TranslatedText
        | ActionValueKind::SavedPath
        | ActionValueKind::WindowHandle => Err(ActionError::InvalidInput),
    }
}

fn exact_object<'a>(
    input: &'a Value,
    fields: &[&str],
) -> Result<&'a Map<String, Value>, ActionError> {
    let object = input.as_object().ok_or(ActionError::InvalidInput)?;
    if object.len() != fields.len() || !fields.iter().all(|field| object.contains_key(*field)) {
        return Err(ActionError::InvalidInput);
    }
    Ok(object)
}

fn validate_owned_image(input: &Value) -> Result<ActionInput, ActionError> {
    let object = exact_object(input, &["sourceId", "sourceVersion"])?;
    let source_id = object
        .get("sourceId")
        .and_then(Value::as_str)
        .ok_or(ActionError::InvalidInput)?;
    let source_version = object
        .get("sourceVersion")
        .and_then(Value::as_u64)
        .ok_or(ActionError::InvalidInput)?;
    if source_id.is_empty()
        || source_id.len() > MAX_SOURCE_ID_BYTES
        || !source_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ActionError::InvalidInput);
    }
    Ok(ActionInput::OwnedImage {
        source_id: source_id.to_string(),
        source_version,
    })
}

fn validate_text(input: &Value) -> Result<ActionInput, ActionError> {
    let object = exact_object(input, &["text"])?;
    let text = validate_bounded_text(object.get("text"))?;
    Ok(ActionInput::Text(text.to_string()))
}

fn validate_translation_request(input: &Value) -> Result<ActionInput, ActionError> {
    let object = input.as_object().ok_or(ActionError::InvalidInput)?;
    if object.len() < 2
        || object.len() > 3
        || !object.contains_key("text")
        || !object.contains_key("targetLanguage")
        || object
            .keys()
            .any(|field| !matches!(field.as_str(), "text" | "sourceLanguage" | "targetLanguage"))
    {
        return Err(ActionError::InvalidInput);
    }
    let text = validate_bounded_text(object.get("text"))?;
    let target_language = validate_language(object.get("targetLanguage"), false)?;
    let source_language = object
        .get("sourceLanguage")
        .map(|value| validate_language(Some(value), true))
        .transpose()?;
    Ok(ActionInput::Translation {
        text: text.to_string(),
        source_language: source_language.map(str::to_string),
        target_language: target_language.to_string(),
    })
}

fn validate_bounded_text(value: Option<&Value>) -> Result<&str, ActionError> {
    let text = value
        .and_then(Value::as_str)
        .ok_or(ActionError::InvalidInput)?;
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(ActionError::InvalidInput);
    }
    Ok(text)
}

fn validate_language(value: Option<&Value>, allow_auto: bool) -> Result<&str, ActionError> {
    let language = value
        .and_then(Value::as_str)
        .ok_or(ActionError::InvalidInput)?;
    if language == "auto" {
        return allow_auto
            .then_some(language)
            .ok_or(ActionError::InvalidInput);
    }
    if !language.is_empty()
        && language.len() <= 32
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Ok(language);
    }
    Err(ActionError::InvalidInput)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActionHandle {
    request_slot: String,
    generation: u64,
}

#[derive(Clone)]
pub(super) struct ActionCancellation(Arc<AtomicBool>);

impl ActionCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

pub(super) struct PreparedAction {
    descriptor: &'static ActionDescriptor,
    handle: ActionHandle,
    input: ActionInput,
    cancellation: ActionCancellation,
}

impl PreparedAction {
    pub fn descriptor(&self) -> &'static ActionDescriptor {
        self.descriptor
    }

    pub fn handle(&self) -> &ActionHandle {
        &self.handle
    }

    pub fn input(&self) -> &ActionInput {
        &self.input
    }

    pub fn cancellation(&self) -> ActionCancellation {
        self.cancellation.clone()
    }
}

impl fmt::Debug for PreparedAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedAction")
            .field("action_id", &self.descriptor.id)
            .field("input_kind", &self.descriptor.input)
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionError {
    #[error("未知动作")]
    UnknownAction,
    #[error("当前窗口无权执行该动作")]
    Unauthorized,
    #[error("动作参数无效")]
    InvalidInput,
    #[error("动作请求槽无效")]
    InvalidRequestSlot,
    #[error("动作请求已经更新")]
    Superseded,
    #[error("动作正在提交")]
    Busy,
    #[error("动作已经取消")]
    Cancelled,
    #[error("动作不支持取消")]
    NotCancellable,
    #[error("动作生命周期与执行方式不匹配")]
    InvalidMode,
    #[error("动作代次已经耗尽")]
    GenerationExhausted,
    #[error("动作状态锁已损坏")]
    Poisoned,
}

impl ActionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnknownAction => "action_unknown",
            Self::Unauthorized => "action_forbidden",
            Self::InvalidInput => "action_invalid_input",
            Self::InvalidRequestSlot => "action_invalid_request_slot",
            Self::Superseded => "action_superseded",
            Self::Busy => "action_busy",
            Self::Cancelled => "action_cancelled",
            Self::NotCancellable => "action_not_cancellable",
            Self::InvalidMode => "action_invalid_mode",
            Self::GenerationExhausted => "action_generation_exhausted",
            Self::Poisoned => "action_internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActionSlot {
    caller: String,
    request_slot: String,
}

struct ActiveAction {
    generation: u64,
    cancellable: bool,
    phase: ActionPhase,
    cancellation: ActionCancellation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionPhase {
    Pending,
    Committing,
}

#[derive(Default)]
struct RuntimeState {
    next_generation: u64,
    active: HashMap<ActionSlot, ActiveAction>,
}

#[derive(Default)]
pub(super) struct ActionRuntime {
    state: Mutex<RuntimeState>,
}

impl ActionRuntime {
    pub fn begin(
        &self,
        caller_label: &str,
        action_id: &str,
        request_slot: &str,
        input: Value,
    ) -> Result<PreparedAction, ActionError> {
        validate_request_slot(request_slot)?;
        let descriptor = descriptor(action_id).ok_or(ActionError::UnknownAction)?;
        if !action_allowed(crate::ipc_access::caller_kind(caller_label), action_id) {
            return Err(ActionError::Unauthorized);
        }
        let input = validate_input(descriptor.input, &input)?;

        let slot = ActionSlot {
            caller: caller_label.to_string(),
            request_slot: request_slot.to_string(),
        };
        let cancellation = ActionCancellation(Arc::new(AtomicBool::new(false)));
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        if state
            .active
            .get(&slot)
            .is_some_and(|active| active.phase == ActionPhase::Committing)
        {
            return Err(ActionError::Busy);
        }
        let generation = state
            .next_generation
            .checked_add(1)
            .ok_or(ActionError::GenerationExhausted)?;
        state.next_generation = generation;
        if let Some(previous) = state.active.insert(
            slot,
            ActiveAction {
                generation,
                cancellable: descriptor.cancellable,
                phase: ActionPhase::Pending,
                cancellation: cancellation.clone(),
            },
        ) {
            previous.cancellation.cancel();
        }
        Ok(PreparedAction {
            descriptor,
            handle: ActionHandle {
                request_slot: request_slot.to_string(),
                generation,
            },
            input,
            cancellation,
        })
    }

    pub fn cancel(&self, caller_label: &str, handle: &ActionHandle) -> Result<(), ActionError> {
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let active = current_action_mut(&mut state, caller_label, handle)?;
        if !active.cancellable {
            return Err(ActionError::NotCancellable);
        }
        active.cancellation.cancel();
        Ok(())
    }

    pub fn ensure_current(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<(), ActionError> {
        let state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let active = current_action(&state, caller_label, handle)?;
        if active.cancellation.is_cancelled() {
            return Err(ActionError::Cancelled);
        }
        Ok(())
    }

    /// 在状态锁内完成一次短小结果发布，保证同槽的新请求不能插入“核验通过”和“写入结果”之间。
    /// `publish` 不得阻塞、等待 UI 或回调本 runtime；耗时工作必须在调用前完成。
    pub fn publish<T, E>(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
        publish: impl FnOnce() -> Result<T, E>,
    ) -> Result<Result<T, E>, ActionError> {
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let active = current_action(&state, caller_label, handle)?;
        if !active.cancellable {
            return Err(ActionError::InvalidMode);
        }
        let cancelled = active.cancellation.is_cancelled();
        if cancelled {
            state.active.remove(&slot_for(caller_label, handle));
            return Err(ActionError::Cancelled);
        }
        let result = publish();
        state.active.remove(&slot_for(caller_label, handle));
        Ok(result)
    }

    /// 不可取消副作用先原子进入提交阶段，再离开状态锁执行。
    /// 提交期间同一 caller/slot 不能被替换，避免旧复制/保存动作晚于新动作落地。
    pub fn commit_noncancellable<T, E>(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
        commit: impl FnOnce() -> Result<T, E>,
    ) -> Result<Result<T, E>, ActionError> {
        {
            let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
            let active = current_action_mut(&mut state, caller_label, handle)?;
            if active.cancellable {
                return Err(ActionError::InvalidMode);
            }
            if active.phase == ActionPhase::Committing {
                return Err(ActionError::Busy);
            }
            active.phase = ActionPhase::Committing;
        }

        let result = commit();
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        current_action(&state, caller_label, handle)?;
        state.active.remove(&slot_for(caller_label, handle));
        Ok(result)
    }
}

fn validate_request_slot(request_slot: &str) -> Result<(), ActionError> {
    if request_slot.is_empty()
        || request_slot.len() > MAX_REQUEST_SLOT_BYTES
        || !request_slot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ActionError::InvalidRequestSlot);
    }
    Ok(())
}

fn slot_for(caller_label: &str, handle: &ActionHandle) -> ActionSlot {
    ActionSlot {
        caller: caller_label.to_string(),
        request_slot: handle.request_slot.clone(),
    }
}

fn current_action<'a>(
    state: &'a RuntimeState,
    caller_label: &str,
    handle: &ActionHandle,
) -> Result<&'a ActiveAction, ActionError> {
    match state.active.get(&slot_for(caller_label, handle)) {
        Some(active) if active.generation == handle.generation => Ok(active),
        _ => Err(ActionError::Superseded),
    }
}

fn current_action_mut<'a>(
    state: &'a mut RuntimeState,
    caller_label: &str,
    handle: &ActionHandle,
) -> Result<&'a mut ActiveAction, ActionError> {
    match state.active.get_mut(&slot_for(caller_label, handle)) {
        Some(active) if active.generation == handle.generation => Ok(active),
        _ => Err(ActionError::Superseded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_input(descriptor: &ActionDescriptor) -> Value {
        match descriptor.input {
            ActionValueKind::Unit => json!({}),
            ActionValueKind::OwnedImage => {
                json!({"sourceId": "viewer-7-revision", "sourceVersion": 3})
            }
            ActionValueKind::Text => json!({"text": "private clipboard text"}),
            ActionValueKind::TranslationRequest => json!({
                "text": "private translation text",
                "sourceLanguage": "auto",
                "targetLanguage": "zh-CN"
            }),
            _ => unreachable!("内置动作输入不能使用输出类型"),
        }
    }

    #[test]
    fn catalog_is_unique_sorted_serializable_and_has_no_process_permission() {
        let ids = descriptors()
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids, sorted);
        assert_eq!(ids.len(), 7);
        let encoded = serde_json::to_value(descriptors()).unwrap();
        assert_eq!(encoded[0]["id"], "capture.start");
        assert_eq!(encoded[0]["permissions"][0], "screen.capture");
        assert_eq!(encoded[6]["permissions"][0], "translation.network");
        assert!(!encoded.to_string().contains("shell"));
        assert!(!encoded.to_string().contains("process"));
        assert!(!encoded.to_string().contains("url"));
    }

    #[test]
    fn every_action_accepts_its_exact_bounded_input_and_rejects_unknown_fields() {
        for action in descriptors() {
            let input = valid_input(action);
            validate_input(action.input, &input).unwrap();
            let mut invalid = input.as_object().unwrap().clone();
            invalid.insert("unexpected".to_string(), json!(true));
            assert!(
                matches!(
                    validate_input(action.input, &Value::Object(invalid)),
                    Err(ActionError::InvalidInput)
                ),
                "{}",
                action.id
            );
        }
        assert!(matches!(
            validate_owned_image(&json!({
                "sourceId": "../../private/image.png",
                "sourceVersion": 1
            })),
            Err(ActionError::InvalidInput)
        ));
        let ActionInput::OwnedImage {
            source_id,
            source_version,
        } = validate_owned_image(&json!({"sourceId": "viewer-initial", "sourceVersion": 0}))
            .unwrap()
        else {
            panic!("owned image input must remain typed");
        };
        assert_eq!(source_id, "viewer-initial");
        assert_eq!(source_version, 0);
        let ActionInput::Translation {
            text,
            source_language,
            target_language,
        } = validate_translation_request(&json!({
            "text": "private",
            "sourceLanguage": "auto",
            "targetLanguage": "zh-CN"
        }))
        .unwrap()
        else {
            panic!("translation input must remain typed");
        };
        assert_eq!(text, "private");
        assert_eq!(source_language.as_deref(), Some("auto"));
        assert_eq!(target_language, "zh-CN");
        assert!(matches!(
            validate_translation_request(&json!({
                "text": "x",
                "sourceLanguage": "auto",
                "targetLanguage": "https://example.com"
            })),
            Err(ActionError::InvalidInput)
        ));
        assert!(matches!(
            validate_translation_request(&json!({
                "text": "x",
                "targetLanguage": "auto"
            })),
            Err(ActionError::InvalidInput)
        ));
        assert!(matches!(
            validate_text(&json!({"text": "x".repeat(MAX_TEXT_BYTES + 1)})),
            Err(ActionError::InvalidInput)
        ));
    }

    #[test]
    fn child_roles_only_receive_their_declared_action_subset() {
        for action in descriptors() {
            assert!(action_allowed(CallerKind::Main, action.id));
            assert!(action_allowed(CallerKind::Launcher, action.id));
            assert!(!action_allowed(CallerKind::Settings, action.id));
            assert!(!action_allowed(CallerKind::LongshotController, action.id));
            assert!(!action_allowed(CallerKind::RecordingControl, action.id));
        }
        assert!(!action_allowed(CallerKind::CaptureOverlay, "capture.start"));
        assert!(action_allowed(CallerKind::CaptureOverlay, "image.ocr"));
        assert!(!action_allowed(CallerKind::Pin, "image.ocr"));
        assert!(action_allowed(CallerKind::Pin, "image.save"));
        assert!(matches!(
            ActionRuntime::default().begin(
                "launcher-lookalike",
                "text.copy",
                "copy",
                json!({"text": "private"})
            ),
            Err(ActionError::Unauthorized)
        ));
    }

    #[test]
    fn prepared_debug_redacts_text_and_owned_image_values() {
        let runtime = ActionRuntime::default();
        for (action, input, secret) in [
            (
                "text.copy",
                json!({"text": "do-not-log-this-text"}),
                "do-not-log-this-text",
            ),
            (
                "image.ocr",
                json!({"sourceId": "do-not-log-source-id", "sourceVersion": 4}),
                "do-not-log-source-id",
            ),
        ] {
            let prepared = runtime.begin("main", action, action, input).unwrap();
            let debug = format!("{prepared:?}");
            assert!(debug.contains(action));
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn replacement_cancels_old_work_and_late_publish_cannot_remove_new_generation() {
        let runtime = ActionRuntime::default();
        let first = runtime
            .begin(
                "image-viewer-one",
                "image.ocr",
                "analysis",
                json!({"sourceId": "source-a", "sourceVersion": 1}),
            )
            .unwrap();
        let first_cancel = first.cancellation();
        let second = runtime
            .begin(
                "image-viewer-one",
                "image.ocr",
                "analysis",
                json!({"sourceId": "source-b", "sourceVersion": 2}),
            )
            .unwrap();
        assert!(first_cancel.is_cancelled());
        assert_eq!(
            runtime.publish("image-viewer-one", first.handle(), || Ok::<_, ()>("old")),
            Err(ActionError::Superseded)
        );
        runtime
            .ensure_current("image-viewer-one", second.handle())
            .unwrap();
        assert_eq!(
            runtime
                .publish("image-viewer-one", second.handle(), || Ok::<_, ()>("new"))
                .unwrap(),
            Ok("new")
        );
        assert_eq!(
            runtime.ensure_current("image-viewer-one", second.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn explicit_cancel_is_idempotent_and_observed_before_publication() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate.preview",
                json!({"text": "secret", "targetLanguage": "ja"}),
            )
            .unwrap();
        let cancellation = prepared.cancellation();
        runtime.cancel("launcher", prepared.handle()).unwrap();
        runtime.cancel("launcher", prepared.handle()).unwrap();
        assert!(cancellation.is_cancelled());
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Cancelled)
        );
        let mut published = false;
        assert_eq!(
            runtime.publish("launcher", prepared.handle(), || {
                published = true;
                Ok::<_, ()>(())
            }),
            Err(ActionError::Cancelled)
        );
        assert!(!published);
        assert_eq!(
            runtime.cancel("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn non_cancellable_action_rejects_cancel_and_blocks_replacement_while_committing() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "text.copy", "copy", json!({"text": "private"}))
            .unwrap();
        assert_eq!(
            runtime.cancel("launcher", prepared.handle()),
            Err(ActionError::NotCancellable)
        );
        runtime
            .ensure_current("launcher", prepared.handle())
            .unwrap();
        assert_eq!(
            runtime
                .commit_noncancellable("launcher", prepared.handle(), || {
                    assert!(matches!(
                        runtime.begin("launcher", "text.copy", "copy", json!({"text": "newer"})),
                        Err(ActionError::Busy)
                    ));
                    Ok::<_, ()>("copied")
                })
                .unwrap(),
            Ok("copied")
        );
        runtime
            .begin("launcher", "text.copy", "copy", json!({"text": "newer"}))
            .unwrap();
        assert_eq!(ActionError::NotCancellable.code(), "action_not_cancellable");
        assert_eq!(ActionError::Busy.code(), "action_busy");
    }

    #[test]
    fn business_error_is_returned_once_and_retires_the_request_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "main",
                "image.ocr",
                "analysis",
                json!({"sourceId": "clip-9", "sourceVersion": 0}),
            )
            .unwrap();
        assert_eq!(
            runtime
                .publish("main", prepared.handle(), || Err::<(), _>("ocr_failed"))
                .unwrap(),
            Err("ocr_failed")
        );
        assert_eq!(
            runtime.ensure_current("main", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn caller_and_request_slot_are_part_of_the_result_identity() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "main",
                "text.copy",
                "clipboard.primary",
                json!({"text": "private"}),
            )
            .unwrap();
        assert_eq!(
            runtime.ensure_current("image-viewer-main", prepared.handle()),
            Err(ActionError::Superseded)
        );
        let forged = ActionHandle {
            request_slot: "clipboard.secondary".to_string(),
            generation: prepared.handle().generation,
        };
        assert_eq!(
            runtime.ensure_current("main", &forged),
            Err(ActionError::Superseded)
        );
        assert_eq!(ActionError::Unauthorized.code(), "action_forbidden");
    }

    #[test]
    fn invalid_action_slot_and_permission_fail_before_allocating_a_generation() {
        let runtime = ActionRuntime::default();
        assert!(matches!(
            runtime.begin("settings", "text.copy", "copy", json!({"text": "secret"})),
            Err(ActionError::Unauthorized)
        ));
        assert!(matches!(
            runtime.begin("main", "unknown.action", "copy", json!({})),
            Err(ActionError::UnknownAction)
        ));
        assert!(matches!(
            runtime.begin("main", "text.copy", "../copy", json!({"text": "secret"})),
            Err(ActionError::InvalidRequestSlot)
        ));
        let first = runtime
            .begin("main", "text.copy", "copy", json!({"text": "ok"}))
            .unwrap();
        assert_eq!(first.handle().generation, 1);
    }
}
