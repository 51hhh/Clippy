//! PX-ACT-01 类型化内置动作核心。
//!
//! 该模块建立静态注册表、参数边界、窗口角色权限和请求代次，并分阶段接入复用既有业务服务的
//! 领域适配器、受限 IPC 与独立启动器。启动器只展示能构造可信输入的动作；动作参数不接受
//! 路径、URL、前端像素或可执行命令。

mod adapters;
pub(crate) mod launcher;

pub(crate) use launcher::open;

use crate::ipc_access::CallerKind;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_SOURCE_ID_BYTES: usize = 128;
const MAX_REQUEST_SLOT_BYTES: usize = 96;
const MAX_ACTIVE_SLOTS_PER_CALLER: usize = 16;
const MAX_ACTIVE_ACTIONS: usize = 64;
const MAX_COMPLETED_ACTIONS_PER_CALLER: usize = 16;
const MAX_OWNED_IMAGE_SOURCES_PER_CALLER: usize = 2;
const MAX_OWNED_IMAGE_SOURCE_BYTES: usize = 64 * 1024 * 1024;
/// JavaScript IPC 精确整数上限；句柄不能在 JSON 桥两端悄悄改变 generation。
const MAX_ACTION_GENERATION: u64 = 9_007_199_254_740_991;
const ALL_PLATFORMS: &[ActionPlatform] = &[
    ActionPlatform::Linux,
    ActionPlatform::Windows,
    ActionPlatform::Macos,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionValueKind {
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
pub(crate) enum ActionPermission {
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
pub(crate) enum ActionPlatform {
    Linux,
    Windows,
    Macos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionDescriptor {
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

#[derive(Clone)]
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
            "image.ocr" | "image.pin" | "image.save" | "image.scan_codes" | "text.copy"
        ),
        CallerKind::Pin => matches!(action_id, "image.save" | "text.copy"),
        CallerKind::Settings
        | CallerKind::RecordingOverlay
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActionHandle {
    request_slot: String,
    generation: u64,
}

struct CancellationState {
    cancelled: AtomicBool,
    signal: tokio::sync::Notify,
}

#[derive(Clone)]
pub(super) struct ActionCancellation(Arc<CancellationState>);

impl ActionCancellation {
    fn new() -> Self {
        Self(Arc::new(CancellationState {
            cancelled: AtomicBool::new(false),
            signal: tokio::sync::Notify::new(),
        }))
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// 让异步领域适配器立即放弃自己的等待者；底层领域服务负责终止、回收或在既有资源预算内
    /// 完成不可中断的工作，动作 generation 闸门统一拒绝迟到结果。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.0.signal.notified().await;
    }

    fn cancel(&self) {
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            // notify_one 会在尚无等待者时保留一个 permit，避免 begin 后、适配器 await 前取消丢信号。
            self.0.signal.notify_one();
        }
    }
}

pub(super) struct PreparedAction {
    descriptor: &'static ActionDescriptor,
    handle: ActionHandle,
    input: ActionInput,
    cancellation: ActionCancellation,
    root_id: u64,
    caller_epoch: u64,
    root_security: ActionRootSecurity,
    owned_image: Option<Arc<Vec<u8>>>,
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

    fn root_id(&self) -> u64 {
        self.root_id
    }

    pub(super) fn source_content_hash(&self) -> Option<&str> {
        self.root_security.content_hash.as_deref()
    }

    pub(super) fn owned_image(&self) -> Option<Arc<Vec<u8>>> {
        self.owned_image.as_ref().map(Arc::clone)
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
    #[error("上游动作结果不可用")]
    UpstreamUnavailable,
    #[error("动作结果类型不能连接到目标动作")]
    IncompatibleOutput,
    #[error("图片来源超出动作预算")]
    ImageSourceTooLarge,
    #[error("敏感内容不能进入联网动作")]
    SensitiveContent,
    #[error("动作状态锁已损坏")]
    Poisoned,
    #[error("动作执行任务异常终止")]
    WorkerFailed,
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
            Self::UpstreamUnavailable => "action_upstream_unavailable",
            Self::IncompatibleOutput => "action_incompatible_output",
            Self::ImageSourceTooLarge => "action_image_source_too_large",
            Self::SensitiveContent => "action_translation_sensitive_content",
            Self::Poisoned => "action_internal",
            Self::WorkerFailed => "action_internal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActionSlot {
    caller: String,
    request_slot: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CompletedActionKey {
    caller: String,
    request_slot: String,
    generation: u64,
}

#[derive(Clone)]
enum CompletedActionValue {
    RecognizedText(String),
    TranslatedText(String),
}

struct CompletedAction {
    root_id: u64,
    root_security: ActionRootSecurity,
    completed_order: u64,
    value: CompletedActionValue,
}

struct OwnedImageSource {
    root_id: u64,
    root_security: ActionRootSecurity,
    source_version: u64,
    png: Arc<Vec<u8>>,
}

#[derive(Clone, Default)]
struct ActionRootSecurity {
    sensitive: bool,
    content_hash: Option<String>,
}

struct ActionRoot {
    id: u64,
    security: ActionRootSecurity,
    owned_image: Option<Arc<Vec<u8>>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LauncherImageSelection {
    #[default]
    Latest,
    None,
    Clip(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnedImageReference {
    pub source_id: String,
    pub source_version: u64,
}

struct ActiveAction {
    generation: u64,
    root_id: u64,
    caller_epoch: u64,
    root_security: ActionRootSecurity,
    owned_image: Option<Arc<Vec<u8>>>,
    cancellable: bool,
    phase: ActionPhase,
    cancellation: ActionCancellation,
    descriptor: &'static ActionDescriptor,
    input: ActionInput,
    claimed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionPhase {
    Pending,
    Committing,
}

#[derive(Default)]
struct RuntimeState {
    next_generation: u64,
    next_identity: u64,
    completed_order: u64,
    active: HashMap<ActionSlot, ActiveAction>,
    completed: HashMap<CompletedActionKey, CompletedAction>,
    owned_images: HashMap<(String, String), OwnedImageSource>,
    caller_epochs: HashMap<String, u64>,
    launcher_image_selection: LauncherImageSelection,
}

#[derive(Clone, Default)]
pub(crate) struct ActionRuntime {
    state: Arc<Mutex<RuntimeState>>,
}

struct NoncancellableCommitGuard {
    runtime: ActionRuntime,
    caller_label: String,
    handle: ActionHandle,
    retired: bool,
}

impl NoncancellableCommitGuard {
    fn new(runtime: ActionRuntime, caller_label: String, handle: ActionHandle) -> Self {
        Self {
            runtime,
            caller_label,
            handle,
            retired: false,
        }
    }

    fn finish(mut self) -> Result<(), ActionError> {
        self.runtime
            .finish_noncancellable_commit(&self.caller_label, &self.handle)?;
        self.retired = true;
        Ok(())
    }
}

impl Drop for NoncancellableCommitGuard {
    fn drop(&mut self) {
        if !self.retired {
            self.runtime
                .abandon_noncancellable_commit(&self.caller_label, &self.handle);
        }
    }
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
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let root = match &input {
            ActionInput::OwnedImage {
                source_id,
                source_version,
            } => match state
                .owned_images
                .get(&(caller_label.to_string(), source_id.clone()))
            {
                Some(source) if source.source_version == *source_version => ActionRoot {
                    id: source.root_id,
                    security: source.root_security.clone(),
                    owned_image: Some(Arc::clone(&source.png)),
                },
                Some(_) => return Err(ActionError::UpstreamUnavailable),
                None if matches!(
                    crate::ipc_access::caller_kind(caller_label),
                    CallerKind::Main | CallerKind::Launcher
                ) =>
                {
                    return Err(ActionError::UpstreamUnavailable);
                }
                None => ActionRoot {
                    id: next_identity(&mut state)?,
                    security: ActionRootSecurity::default(),
                    owned_image: None,
                },
            },
            _ => ActionRoot {
                id: next_identity(&mut state)?,
                security: ActionRootSecurity::default(),
                owned_image: None,
            },
        };
        begin_locked(
            &mut state,
            caller_label,
            descriptor,
            request_slot,
            input,
            root,
        )
    }

    /// 组合动作只引用后端保存的精确上游 handle。正文从完成结果派生，前端只能提交
    /// 目标动作所需的非内容选项，因此不能替换 OCR 或译文。
    pub fn compose(
        &self,
        caller_label: &str,
        upstream: &ActionHandle,
        action_id: &str,
        request_slot: &str,
        options: Value,
    ) -> Result<PreparedAction, ActionError> {
        validate_handle(upstream)?;
        validate_request_slot(request_slot)?;
        let descriptor = descriptor(action_id).ok_or(ActionError::UnknownAction)?;
        if !action_allowed(crate::ipc_access::caller_kind(caller_label), action_id) {
            return Err(ActionError::Unauthorized);
        }
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let completed = state
            .completed
            .get(&completed_key(caller_label, upstream))
            .ok_or(ActionError::UpstreamUnavailable)?;
        let input = composed_input(&completed.value, descriptor, &options)?;
        let root = ActionRoot {
            id: completed.root_id,
            security: completed.root_security.clone(),
            owned_image: None,
        };
        if descriptor.id == "text.translate" && root.security.sensitive {
            return Err(ActionError::SensitiveContent);
        }
        begin_locked(
            &mut state,
            caller_label,
            descriptor,
            request_slot,
            input,
            root,
        )
    }

    pub(crate) fn set_launcher_image_selection(&self, selection: LauncherImageSelection) {
        if let Ok(mut state) = self.state.lock() {
            state.launcher_image_selection = selection;
        }
    }

    pub(crate) fn launcher_image_selection(&self) -> Result<LauncherImageSelection, ActionError> {
        self.state
            .lock()
            .map(|state| state.launcher_image_selection)
            .map_err(|_| ActionError::Poisoned)
    }

    /// 把后端读取并校验过的不可变 PNG 绑定到真实窗口。每个窗口只保留一个当前来源和一个
    /// 短暂旧来源，避免 UI 重载竞态；单图受 64 MiB 上限约束。
    #[cfg(test)]
    pub(crate) fn register_owned_image(
        &self,
        caller_label: &str,
        png: Vec<u8>,
    ) -> Result<OwnedImageReference, ActionError> {
        let caller_epoch = self.caller_epoch(caller_label)?;
        self.register_owned_image_at_epoch(caller_label, caller_epoch, png, false, None)
    }

    pub(crate) fn caller_epoch(&self, caller_label: &str) -> Result<u64, ActionError> {
        self.state
            .lock()
            .map(|state| {
                state
                    .caller_epochs
                    .get(caller_label)
                    .copied()
                    .unwrap_or_default()
            })
            .map_err(|_| ActionError::Poisoned)
    }

    pub(crate) fn register_owned_image_at_epoch(
        &self,
        caller_label: &str,
        caller_epoch: u64,
        png: Vec<u8>,
        sensitive: bool,
        content_hash: Option<String>,
    ) -> Result<OwnedImageReference, ActionError> {
        if png.is_empty() || png.len() > MAX_OWNED_IMAGE_SOURCE_BYTES {
            return Err(ActionError::ImageSourceTooLarge);
        }
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        if state
            .caller_epochs
            .get(caller_label)
            .copied()
            .unwrap_or_default()
            != caller_epoch
        {
            return Err(ActionError::UpstreamUnavailable);
        }
        if state.active.iter().any(|(slot, active)| {
            slot.caller == caller_label && matches!(active.input, ActionInput::OwnedImage { .. })
        }) {
            return Err(ActionError::Busy);
        }
        let root_id = next_identity(&mut state)?;
        let source_id = format!("action-image-{root_id}");
        let source_version = 0;
        let mut caller_sources = state
            .owned_images
            .iter()
            .filter_map(|((caller, source_id), source)| {
                (caller == caller_label).then_some((source_id.clone(), source.root_id))
            })
            .collect::<Vec<_>>();
        caller_sources.sort_unstable_by_key(|(_, root)| *root);
        while caller_sources.len() >= MAX_OWNED_IMAGE_SOURCES_PER_CALLER {
            let (old_id, _) = caller_sources.remove(0);
            state
                .owned_images
                .remove(&(caller_label.to_string(), old_id));
        }
        state.owned_images.insert(
            (caller_label.to_string(), source_id.clone()),
            OwnedImageSource {
                root_id,
                root_security: ActionRootSecurity {
                    sensitive,
                    content_hash,
                },
                source_version,
                png: Arc::new(png),
            },
        );
        Ok(OwnedImageReference {
            source_id,
            source_version,
        })
    }

    #[cfg(test)]
    pub(super) fn resolve_owned_image(
        &self,
        caller_label: &str,
        source_id: &str,
        source_version: u64,
    ) -> Result<Option<Arc<Vec<u8>>>, ActionError> {
        let state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        Ok(state
            .owned_images
            .get(&(caller_label.to_string(), source_id.to_string()))
            .filter(|source| source.source_version == source_version)
            .map(|source| Arc::clone(&source.png)))
    }

    fn record_completed(
        &self,
        caller_label: &str,
        prepared: &PreparedAction,
        output: &ActionOutput,
    ) -> Result<(), ActionError> {
        let value = match output {
            ActionOutput::RecognizedText(result) => {
                if validate_derived_text(&result.text).is_err() {
                    return Ok(());
                }
                CompletedActionValue::RecognizedText(result.text.clone())
            }
            ActionOutput::TranslatedText(result) => {
                if validate_derived_text(&result.translated_text).is_err() {
                    return Ok(());
                }
                CompletedActionValue::TranslatedText(result.translated_text.clone())
            }
            _ => return Ok(()),
        };
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        if state
            .caller_epochs
            .get(caller_label)
            .copied()
            .unwrap_or_default()
            != prepared.caller_epoch
        {
            return Ok(());
        }
        state.completed_order = state
            .completed_order
            .checked_add(1)
            .filter(|value| *value <= MAX_ACTION_GENERATION)
            .ok_or(ActionError::GenerationExhausted)?;
        let completed_order = state.completed_order;
        let caller_completed = state
            .completed
            .iter()
            .filter_map(|(key, value)| {
                (key.caller == caller_label).then_some((key.clone(), value.completed_order))
            })
            .collect::<Vec<_>>();
        if caller_completed.len() >= MAX_COMPLETED_ACTIONS_PER_CALLER {
            if let Some((oldest, _)) = caller_completed.into_iter().min_by_key(|(_, order)| *order)
            {
                state.completed.remove(&oldest);
            }
        }
        state.completed.insert(
            completed_key(caller_label, prepared.handle()),
            CompletedAction {
                root_id: prepared.root_id(),
                root_security: prepared.root_security.clone(),
                completed_order,
                value,
            },
        );
        Ok(())
    }

    #[cfg(test)]
    fn root_for_completed(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<u64, ActionError> {
        let state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        state
            .completed
            .get(&completed_key(caller_label, handle))
            .map(|completed| completed.root_id)
            .ok_or(ActionError::UpstreamUnavailable)
    }

    #[cfg(test)]
    fn completed_text(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<String, ActionError> {
        let state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        match &state
            .completed
            .get(&completed_key(caller_label, handle))
            .ok_or(ActionError::UpstreamUnavailable)?
            .value
        {
            CompletedActionValue::RecognizedText(text)
            | CompletedActionValue::TranslatedText(text) => Ok(text.clone()),
        }
    }

    /// IPC 的 prepare/run 两阶段只把 handle 交给前端；执行时从当前槽取回第一次
    /// 验证后的类型化输入。一个 handle 只能领取一次，重复 invoke 不能重做副作用。
    pub fn claim_prepared(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<PreparedAction, ActionError> {
        validate_handle(handle)?;
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let slot = slot_for(caller_label, handle);
        if current_action(&state, caller_label, handle)?
            .cancellation
            .is_cancelled()
        {
            state.active.remove(&slot);
            return Err(ActionError::Cancelled);
        }
        let active = current_action_mut(&mut state, caller_label, handle)?;
        if active.claimed {
            return Err(ActionError::Busy);
        }
        active.claimed = true;
        Ok(PreparedAction {
            descriptor: active.descriptor,
            handle: handle.clone(),
            input: active.input.clone(),
            cancellation: active.cancellation.clone(),
            root_id: active.root_id,
            caller_epoch: active.caller_epoch,
            root_security: active.root_security.clone(),
            owned_image: active.owned_image.as_ref().map(Arc::clone),
        })
    }

    pub fn cancel(&self, caller_label: &str, handle: &ActionHandle) -> Result<(), ActionError> {
        validate_handle(handle)?;
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
        self.begin_noncancellable_commit(caller_label, handle)?;
        let guard =
            NoncancellableCommitGuard::new(self.clone(), caller_label.to_string(), handle.clone());
        let result = commit();
        guard.finish()?;
        Ok(result)
    }

    /// 不可取消的异步副作用在独立任务中完成。调用方 future 即使因窗口关闭而被丢弃，
    /// 截图启动仍会走到领域层成功或补偿终点，随后回收精确动作槽。
    pub async fn commit_noncancellable_async<T, E, Start, StartFuture>(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
        start: Start,
    ) -> Result<Result<T, E>, ActionError>
    where
        T: Send + 'static,
        E: Send + 'static,
        Start: FnOnce() -> StartFuture + Send + 'static,
        StartFuture: std::future::Future<Output = Result<T, E>> + Send + 'static,
    {
        self.begin_noncancellable_commit(caller_label, handle)?;
        let guard =
            NoncancellableCommitGuard::new(self.clone(), caller_label.to_string(), handle.clone());
        let worker = tokio::spawn(async move {
            let result = start().await;
            guard.finish()?;
            Ok::<_, ActionError>(result)
        });
        worker.await.map_err(|_| ActionError::WorkerFailed)?
    }

    fn begin_noncancellable_commit(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<(), ActionError> {
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let active = current_action_mut(&mut state, caller_label, handle)?;
        if active.cancellable {
            return Err(ActionError::InvalidMode);
        }
        if active.phase == ActionPhase::Committing {
            return Err(ActionError::Busy);
        }
        active.phase = ActionPhase::Committing;
        Ok(())
    }

    fn finish_noncancellable_commit(
        &self,
        caller_label: &str,
        handle: &ActionHandle,
    ) -> Result<(), ActionError> {
        let mut state = self.state.lock().map_err(|_| ActionError::Poisoned)?;
        let active = current_action(&state, caller_label, handle)?;
        if active.phase != ActionPhase::Committing {
            return Err(ActionError::InvalidMode);
        }
        state.active.remove(&slot_for(caller_label, handle));
        Ok(())
    }

    fn abandon_noncancellable_commit(&self, caller_label: &str, handle: &ActionHandle) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let slot = slot_for(caller_label, handle);
        if state.active.get(&slot).is_some_and(|active| {
            active.generation == handle.generation && active.phase == ActionPhase::Committing
        }) {
            state.active.remove(&slot);
        }
    }

    fn abandon_claimed(&self, caller_label: &str, handle: &ActionHandle) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let slot = slot_for(caller_label, handle);
        if state.active.get(&slot).is_some_and(|active| {
            active.generation == handle.generation
                && active.claimed
                && active.phase == ActionPhase::Pending
        }) {
            if let Some(active) = state.active.remove(&slot) {
                active.cancellation.cancel();
            }
        }
    }

    /// 窗口销毁时回收尚未提交的 prepare/run 状态；已进入不可取消提交的领域任务保留到 guard
    /// 完成，避免源窗口恢复、文件落盘或原生建窗只执行一半。
    pub(crate) fn retire_caller_pending(&self, caller_label: &str) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let slots = state
            .active
            .iter()
            .filter_map(|(slot, active)| {
                (slot.caller == caller_label && active.phase == ActionPhase::Pending)
                    .then_some(slot.clone())
            })
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(active) = state.active.remove(&slot) {
                active.cancellation.cancel();
            }
        }
        state.completed.retain(|key, _| key.caller != caller_label);
        state
            .owned_images
            .retain(|(caller, _), _| caller != caller_label);
        let epoch = state
            .caller_epochs
            .entry(caller_label.to_string())
            .or_default();
        *epoch = epoch.saturating_add(1);
    }
}

fn next_identity(state: &mut RuntimeState) -> Result<u64, ActionError> {
    let identity = state
        .next_identity
        .checked_add(1)
        .filter(|value| *value <= MAX_ACTION_GENERATION)
        .ok_or(ActionError::GenerationExhausted)?;
    state.next_identity = identity;
    Ok(identity)
}

fn begin_locked(
    state: &mut RuntimeState,
    caller_label: &str,
    descriptor: &'static ActionDescriptor,
    request_slot: &str,
    input: ActionInput,
    root: ActionRoot,
) -> Result<PreparedAction, ActionError> {
    let caller_epoch = state
        .caller_epochs
        .get(caller_label)
        .copied()
        .unwrap_or_default();
    let slot = ActionSlot {
        caller: caller_label.to_string(),
        request_slot: request_slot.to_string(),
    };
    if state
        .active
        .get(&slot)
        .is_some_and(|active| active.phase == ActionPhase::Committing)
    {
        return Err(ActionError::Busy);
    }
    if !state.active.contains_key(&slot)
        && (state.active.len() >= MAX_ACTIVE_ACTIONS
            || state
                .active
                .keys()
                .filter(|active_slot| active_slot.caller == caller_label)
                .count()
                >= MAX_ACTIVE_SLOTS_PER_CALLER)
    {
        return Err(ActionError::Busy);
    }
    let generation = state
        .next_generation
        .checked_add(1)
        .filter(|value| *value <= MAX_ACTION_GENERATION)
        .ok_or(ActionError::GenerationExhausted)?;
    state.next_generation = generation;
    let cancellation = ActionCancellation::new();
    if let Some(previous) = state.active.insert(
        slot,
        ActiveAction {
            generation,
            root_id: root.id,
            caller_epoch,
            root_security: root.security.clone(),
            owned_image: root.owned_image.as_ref().map(Arc::clone),
            cancellable: descriptor.cancellable,
            phase: ActionPhase::Pending,
            cancellation: cancellation.clone(),
            descriptor,
            input: input.clone(),
            claimed: false,
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
        root_id: root.id,
        caller_epoch,
        root_security: root.security,
        owned_image: root.owned_image,
    })
}

fn completed_key(caller_label: &str, handle: &ActionHandle) -> CompletedActionKey {
    CompletedActionKey {
        caller: caller_label.to_string(),
        request_slot: handle.request_slot.clone(),
        generation: handle.generation,
    }
}

fn validate_derived_text(text: &str) -> Result<(), ActionError> {
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(ActionError::IncompatibleOutput);
    }
    Ok(())
}

fn composed_input(
    upstream: &CompletedActionValue,
    descriptor: &'static ActionDescriptor,
    options: &Value,
) -> Result<ActionInput, ActionError> {
    let text = match upstream {
        CompletedActionValue::RecognizedText(text) | CompletedActionValue::TranslatedText(text) => {
            text
        }
    };
    validate_derived_text(text)?;
    match (upstream, descriptor.id) {
        (CompletedActionValue::RecognizedText(_), "text.copy")
        | (CompletedActionValue::TranslatedText(_), "text.copy") => {
            exact_object(options, &[])?;
            Ok(ActionInput::Text(text.clone()))
        }
        (CompletedActionValue::RecognizedText(_), "text.translate") => {
            let object = options.as_object().ok_or(ActionError::InvalidInput)?;
            if object.is_empty()
                || object.len() > 2
                || !object.contains_key("targetLanguage")
                || object
                    .keys()
                    .any(|field| !matches!(field.as_str(), "sourceLanguage" | "targetLanguage"))
            {
                return Err(ActionError::InvalidInput);
            }
            let source_language = object
                .get("sourceLanguage")
                .map(|value| validate_language(Some(value), true))
                .transpose()?;
            let target_language = validate_language(object.get("targetLanguage"), false)?;
            Ok(ActionInput::Translation {
                text: text.clone(),
                source_language: source_language.map(str::to_string),
                target_language: target_language.to_string(),
            })
        }
        _ => Err(ActionError::IncompatibleOutput),
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

fn validate_handle(handle: &ActionHandle) -> Result<(), ActionError> {
    validate_request_slot(&handle.request_slot)?;
    if handle.generation == 0 || handle.generation > MAX_ACTION_GENERATION {
        return Err(ActionError::InvalidInput);
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

/// IPC 只暴露稳定错误码；输入正文、图片身份、路径和底层错误都不会进入响应。
#[derive(Debug, Serialize)]
pub(crate) struct ActionIpcError {
    code: &'static str,
}

impl From<ActionError> for ActionIpcError {
    fn from(error: ActionError) -> Self {
        Self { code: error.code() }
    }
}

impl From<adapters::ActionRunError> for ActionIpcError {
    fn from(error: adapters::ActionRunError) -> Self {
        Self { code: error.code() }
    }
}

#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(crate) enum ActionOutput {
    Unit,
    CaptureSession(String),
    RecognizedText(crate::ocr::StructuredOcr),
    DetectedCodes(crate::code_detection::CodeScanResponse),
    TranslatedText(crate::translation::types::TranslationResult),
    SavedPath(String),
    WindowHandle(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActionReply {
    handle: ActionHandle,
    output: ActionOutput,
}

struct ClaimedActionGuard {
    runtime: ActionRuntime,
    caller_label: String,
    handle: ActionHandle,
}

impl ClaimedActionGuard {
    fn new(runtime: ActionRuntime, caller_label: String, handle: ActionHandle) -> Self {
        Self {
            runtime,
            caller_label,
            handle,
        }
    }
}

impl Drop for ClaimedActionGuard {
    fn drop(&mut self) {
        // 正常完成时领域适配器已经精确回收槽；调用 future 被窗口关闭中断时，这里取消并
        // 回收尚未提交的动作。进入不可取消提交阶段后由提交 guard 负责走到补偿终点。
        self.runtime
            .abandon_claimed(&self.caller_label, &self.handle);
    }
}

fn available_descriptors(caller_label: &str) -> Vec<ActionDescriptor> {
    let caller = crate::ipc_access::caller_kind(caller_label);
    descriptors()
        .iter()
        .copied()
        .filter(|descriptor| action_allowed(caller, descriptor.id))
        .collect()
}

/// 返回当前原生窗口可用的静态动作目录。调用者身份由 Tauri 注入，前端不能自报角色。
#[tauri::command]
pub(crate) fn discover_actions(window: tauri::WebviewWindow) -> Vec<ActionDescriptor> {
    available_descriptors(window.label())
}

/// 第一次也是唯一一次接收动作输入；校验后的类型化值留在 Rust 内存，前端只拿到句柄。
#[tauri::command]
pub(crate) fn prepare_action(
    action_id: String,
    request_slot: String,
    input: Value,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<ActionHandle, ActionIpcError> {
    let prepared = state
        .action_runtime
        .begin(window.label(), &action_id, &request_slot, input)?;
    Ok(prepared.handle().clone())
}

/// 从后端保存的精确完成结果派生下一动作。`options` 只包含语言等非内容字段，正文不能回传。
#[tauri::command]
pub(crate) fn prepare_composed_action(
    upstream: ActionHandle,
    action_id: String,
    request_slot: String,
    options: Value,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<ActionHandle, ActionIpcError> {
    let prepared = state.action_runtime.compose(
        window.label(),
        &upstream,
        &action_id,
        &request_slot,
        options,
    )?;
    Ok(prepared.handle().clone())
}

/// 只凭后端签发的一次性句柄执行；不能在 run 阶段替换动作 ID 或输入。
#[tauri::command]
pub(crate) async fn run_action(
    handle: ActionHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<ActionReply, ActionIpcError> {
    use tauri::Manager;

    let caller_label = window.label().to_string();
    let runtime = state.action_runtime.clone();
    let prepared = runtime.claim_prepared(&caller_label, &handle)?;
    let _guard = ClaimedActionGuard::new(runtime.clone(), caller_label.clone(), handle.clone());
    let output = match prepared.descriptor().id {
        "capture.start" => ActionOutput::CaptureSession(
            adapters::start_capture(&runtime, &caller_label, &prepared, window.app_handle())
                .await?,
        ),
        "image.ocr" => ActionOutput::RecognizedText(
            adapters::ocr_image(&runtime, &caller_label, &prepared, &state).await?,
        ),
        "image.pin" => ActionOutput::WindowHandle(adapters::pin_image(
            &runtime,
            &caller_label,
            &prepared,
            window.app_handle(),
            &state,
        )?),
        "image.save" => ActionOutput::SavedPath(adapters::save_image(
            &runtime,
            &caller_label,
            &prepared,
            &state,
        )?),
        "image.scan_codes" => ActionOutput::DetectedCodes(
            adapters::scan_image_codes(&runtime, &caller_label, &prepared, &state).await?,
        ),
        "text.copy" => {
            adapters::copy_text(&runtime, &caller_label, &prepared, &state)?;
            ActionOutput::Unit
        }
        "text.translate" => ActionOutput::TranslatedText(
            adapters::translate_text(&runtime, &caller_label, &prepared, &state).await?,
        ),
        _ => return Err(ActionError::UnknownAction.into()),
    };
    runtime.record_completed(&caller_label, &prepared, &output)?;
    Ok(ActionReply { handle, output })
}

/// 取消只能命中当前窗口、当前 request slot、当前 generation 的可取消动作。
#[tauri::command]
pub(crate) fn cancel_action(
    handle: ActionHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<(), ActionIpcError> {
    state.action_runtime.cancel(window.label(), &handle)?;
    Ok(())
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

    fn ocr_output(text: &str) -> ActionOutput {
        ActionOutput::RecognizedText(
            serde_json::from_value(json!({
                "width": 2,
                "height": 1,
                "text": text,
                "lines": [],
                "paragraphs": [],
                "pipeline": {
                    "id": "fixture",
                    "engine": "tesseract",
                    "featureSchema": null,
                    "layoutExecuted": false,
                    "layoutReason": "unstructured_backend"
                },
                "fallbackReason": "fixture"
            }))
            .unwrap(),
        )
    }

    fn translated_output(text: &str) -> ActionOutput {
        ActionOutput::TranslatedText(crate::translation::types::TranslationResult {
            request_id: 1,
            provider: crate::translation::types::TranslationProvider::LibreTranslate,
            translated_text: text.to_string(),
            detected_source_language: Some("en".to_string()),
            target_language: "zh-CN".to_string(),
        })
    }

    #[test]
    fn launcher_owned_image_is_window_bound_and_removed_on_teardown() {
        let runtime = ActionRuntime::default();
        let opening_epoch = runtime.caller_epoch("launcher").unwrap();
        assert!(matches!(
            runtime.begin(
                "launcher",
                "image.ocr",
                "ocr",
                json!({"sourceId": "forged", "sourceVersion": 0}),
            ),
            Err(ActionError::UpstreamUnavailable)
        ));
        let reference = runtime
            .register_owned_image("launcher", vec![1, 2, 3])
            .unwrap();
        assert_eq!(
            runtime
                .resolve_owned_image("launcher", &reference.source_id, reference.source_version)
                .unwrap()
                .unwrap()
                .as_slice(),
            &[1, 2, 3]
        );
        assert!(runtime
            .begin(
                "launcher",
                "image.ocr",
                "ocr",
                json!({
                    "sourceId": reference.source_id,
                    "sourceVersion": reference.source_version + 1
                }),
            )
            .is_err());
        runtime.retire_caller_pending("launcher");
        assert!(runtime
            .resolve_owned_image("launcher", &reference.source_id, reference.source_version)
            .unwrap()
            .is_none());
        assert_eq!(
            runtime.register_owned_image_at_epoch(
                "launcher",
                opening_epoch,
                vec![4, 5, 6],
                false,
                None,
            ),
            Err(ActionError::UpstreamUnavailable)
        );
    }

    #[test]
    fn composition_uses_saved_output_and_preserves_the_root_identity() {
        let runtime = ActionRuntime::default();
        let reference = runtime
            .register_owned_image("launcher", vec![1, 2, 3])
            .unwrap();
        let ocr = runtime
            .begin(
                "launcher",
                "image.ocr",
                "ocr",
                json!({
                    "sourceId": reference.source_id,
                    "sourceVersion": reference.source_version
                }),
            )
            .unwrap();
        runtime
            .publish("launcher", ocr.handle(), || Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        let output = ocr_output("trusted OCR text");
        runtime.record_completed("launcher", &ocr, &output).unwrap();
        let root = runtime
            .root_for_completed("launcher", ocr.handle())
            .unwrap();

        let translate = runtime
            .compose(
                "launcher",
                ocr.handle(),
                "text.translate",
                "translate",
                json!({"sourceLanguage": "auto", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        assert_eq!(translate.root_id(), root);
        assert!(matches!(
            translate.input(),
            ActionInput::Translation { text, target_language, .. }
                if text == "trusted OCR text" && target_language == "zh-CN"
        ));
        runtime
            .publish("launcher", translate.handle(), || Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        let translated = translated_output("可信译文");
        runtime
            .record_completed("launcher", &translate, &translated)
            .unwrap();
        assert_eq!(
            runtime
                .root_for_completed("launcher", translate.handle())
                .unwrap(),
            root
        );
        assert_eq!(
            runtime
                .completed_text("launcher", translate.handle())
                .unwrap(),
            "可信译文"
        );

        let copy = runtime
            .compose(
                "launcher",
                translate.handle(),
                "text.copy",
                "copy",
                json!({}),
            )
            .unwrap();
        assert_eq!(copy.root_id(), root);
        assert!(matches!(copy.input(), ActionInput::Text(text) if text == "可信译文"));
    }

    #[test]
    fn composition_rejects_forged_cross_window_and_incompatible_inputs() {
        let runtime = ActionRuntime::default();
        let translated = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "private", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        runtime
            .publish("launcher", translated.handle(), || Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        runtime
            .record_completed("launcher", &translated, &translated_output("result"))
            .unwrap();
        assert!(matches!(
            runtime.compose("main", translated.handle(), "text.copy", "copy", json!({}),),
            Err(ActionError::UpstreamUnavailable)
        ));
        assert!(matches!(
            runtime.compose(
                "launcher",
                translated.handle(),
                "text.translate",
                "again",
                json!({"targetLanguage": "ja"}),
            ),
            Err(ActionError::IncompatibleOutput)
        ));
        assert!(matches!(
            runtime.compose(
                "launcher",
                translated.handle(),
                "text.copy",
                "copy",
                json!({"text": "forged"}),
            ),
            Err(ActionError::InvalidInput)
        ));
        runtime.retire_caller_pending("launcher");
        assert!(matches!(
            runtime.compose(
                "launcher",
                translated.handle(),
                "text.copy",
                "copy",
                json!({}),
            ),
            Err(ActionError::UpstreamUnavailable)
        ));
    }

    #[test]
    fn destroyed_window_epoch_blocks_a_late_completed_result_from_reappearing() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "private", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        runtime
            .publish("launcher", prepared.handle(), || Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        runtime.retire_caller_pending("launcher");
        runtime
            .record_completed("launcher", &prepared, &translated_output("late"))
            .unwrap();
        assert_eq!(
            runtime.root_for_completed("launcher", prepared.handle()),
            Err(ActionError::UpstreamUnavailable)
        );
    }

    #[test]
    fn sensitive_image_result_can_copy_locally_but_cannot_compose_to_network_translation() {
        let runtime = ActionRuntime::default();
        let epoch = runtime.caller_epoch("launcher").unwrap();
        let reference = runtime
            .register_owned_image_at_epoch(
                "launcher",
                epoch,
                vec![1, 2, 3],
                true,
                Some("private-hash".to_string()),
            )
            .unwrap();
        let ocr = runtime
            .begin(
                "launcher",
                "image.ocr",
                "ocr",
                json!({
                    "sourceId": reference.source_id,
                    "sourceVersion": reference.source_version
                }),
            )
            .unwrap();
        runtime
            .publish("launcher", ocr.handle(), || Ok::<_, ()>(()))
            .unwrap()
            .unwrap();
        runtime
            .record_completed("launcher", &ocr, &ocr_output("private text"))
            .unwrap();
        assert!(runtime
            .compose("launcher", ocr.handle(), "text.copy", "copy", json!({}),)
            .is_ok());
        assert!(matches!(
            runtime.compose(
                "launcher",
                ocr.handle(),
                "text.translate",
                "translate",
                json!({"targetLanguage": "en"}),
            ),
            Err(ActionError::SensitiveContent)
        ));
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
        assert!(action_allowed(CallerKind::ImageViewer, "image.ocr"));
        assert!(!action_allowed(
            CallerKind::CaptureOverlay,
            "text.translate"
        ));
        assert!(!action_allowed(CallerKind::ImageViewer, "text.translate"));
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
        for (caller, action, input, secret) in [
            (
                "main",
                "text.copy",
                json!({"text": "do-not-log-this-text"}),
                "do-not-log-this-text",
            ),
            (
                "image-viewer-owner",
                "image.ocr",
                json!({"sourceId": "do-not-log-source-id", "sourceVersion": 4}),
                "do-not-log-source-id",
            ),
        ] {
            let prepared = runtime.begin(caller, action, action, input).unwrap();
            let debug = format!("{prepared:?}");
            assert!(debug.contains(action));
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn ipc_handle_claim_is_single_use_and_keeps_the_first_validated_input() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate.preview",
                json!({
                    "text": "private  text",
                    "sourceLanguage": "auto",
                    "targetLanguage": "ja"
                }),
            )
            .unwrap();
        let handle = prepared.handle().clone();
        drop(prepared);

        let claimed = runtime.claim_prepared("launcher", &handle).unwrap();
        let ActionInput::Translation {
            text,
            source_language,
            target_language,
        } = claimed.input()
        else {
            panic!("claim must restore the typed input");
        };
        assert_eq!(text, "private  text");
        assert_eq!(source_language.as_deref(), Some("auto"));
        assert_eq!(target_language, "ja");
        assert!(matches!(
            runtime.claim_prepared("launcher", &handle),
            Err(ActionError::Busy)
        ));
        assert!(matches!(
            runtime.claim_prepared("main", &handle),
            Err(ActionError::Superseded)
        ));
    }

    #[test]
    fn cancellation_before_run_consumes_the_handle_without_exposing_input() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate.preview",
                json!({"text": "private", "targetLanguage": "ja"}),
            )
            .unwrap();
        let handle = prepared.handle().clone();
        runtime.cancel("launcher", &handle).unwrap();
        assert!(matches!(
            runtime.claim_prepared("launcher", &handle),
            Err(ActionError::Cancelled)
        ));
        assert!(matches!(
            runtime.claim_prepared("launcher", &handle),
            Err(ActionError::Superseded)
        ));
    }

    #[test]
    fn dropped_run_waiter_cancels_and_retires_a_claimed_pending_action() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.ocr",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let handle = prepared.handle().clone();
        let cancellation = runtime
            .claim_prepared("image-viewer-owner", &handle)
            .unwrap()
            .cancellation();
        drop(ClaimedActionGuard::new(
            runtime.clone(),
            "image-viewer-owner".to_string(),
            handle.clone(),
        ));
        assert!(cancellation.is_cancelled());
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", &handle),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn ipc_handle_and_reply_have_exact_stable_shapes() {
        let handle: ActionHandle = serde_json::from_value(json!({
            "requestSlot": "analysis.primary",
            "generation": 7
        }))
        .unwrap();
        assert_eq!(handle.request_slot, "analysis.primary");
        assert!(serde_json::from_value::<ActionHandle>(json!({
            "requestSlot": "analysis.primary",
            "generation": 7,
            "actionId": "image.ocr"
        }))
        .is_err());
        assert!(matches!(
            validate_handle(&ActionHandle {
                request_slot: "analysis".to_string(),
                generation: 0,
            }),
            Err(ActionError::InvalidInput)
        ));
        assert_eq!(
            serde_json::to_value(ActionReply {
                handle,
                output: ActionOutput::Unit,
            })
            .unwrap(),
            json!({
                "handle": {"requestSlot": "analysis.primary", "generation": 7},
                "output": {"type": "unit"}
            })
        );
        assert_eq!(
            serde_json::to_value(ActionIpcError::from(ActionError::Poisoned)).unwrap(),
            json!({"code": "action_internal"})
        );
    }

    #[test]
    fn discovery_filters_the_static_catalog_by_native_window_role() {
        assert_eq!(available_descriptors("launcher").len(), 7);
        assert_eq!(available_descriptors("image-viewer-one").len(), 5);
        assert_eq!(available_descriptors("capture-overlay-one").len(), 5);
        assert_eq!(available_descriptors("pin-image-one").len(), 2);
        assert!(available_descriptors("settings").is_empty());
        assert!(available_descriptors("launcher-lookalike").is_empty());
    }

    #[test]
    fn prepared_slots_are_bounded_and_window_teardown_reclaims_pending_inputs() {
        let runtime = ActionRuntime::default();
        let mut cancellations = Vec::new();
        for index in 0..MAX_ACTIVE_SLOTS_PER_CALLER {
            let prepared = runtime
                .begin(
                    "launcher",
                    "text.translate",
                    &format!("slot.{index}"),
                    json!({"text": "private", "targetLanguage": "ja"}),
                )
                .unwrap();
            cancellations.push(prepared.cancellation());
        }
        assert!(matches!(
            runtime.begin(
                "launcher",
                "text.translate",
                "slot.overflow",
                json!({"text": "private", "targetLanguage": "ja"})
            ),
            Err(ActionError::Busy)
        ));
        // 替换既有槽不增加内存预算，仍须可用。
        runtime
            .begin(
                "launcher",
                "text.translate",
                "slot.0",
                json!({"text": "new", "targetLanguage": "ja"}),
            )
            .unwrap();
        runtime.retire_caller_pending("launcher");
        assert!(cancellations.into_iter().all(|token| token.is_cancelled()));
        assert!(runtime
            .begin(
                "launcher",
                "text.translate",
                "slot.after-close",
                json!({"text": "private", "targetLanguage": "ja"})
            )
            .is_ok());
        runtime.retire_caller_pending("launcher");

        for caller in 0..(MAX_ACTIVE_ACTIONS / MAX_ACTIVE_SLOTS_PER_CALLER) {
            for slot in 0..MAX_ACTIVE_SLOTS_PER_CALLER {
                runtime
                    .begin(
                        &format!("image-viewer-{caller}"),
                        "image.ocr",
                        &format!("slot.{slot}"),
                        json!({"sourceId": "snapshot", "sourceVersion": 0}),
                    )
                    .unwrap();
            }
        }
        assert!(matches!(
            runtime.begin("main", "capture.start", "global.overflow", json!({})),
            Err(ActionError::Busy)
        ));
        runtime.retire_caller_pending("image-viewer-0");
        assert!(runtime
            .begin("main", "capture.start", "global.reclaimed", json!({}))
            .is_ok());
    }

    #[test]
    fn window_teardown_does_not_interrupt_an_active_noncancellable_commit() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "text.copy", "copy", json!({"text": "private"}))
            .unwrap();
        runtime
            .begin_noncancellable_commit("launcher", prepared.handle())
            .unwrap();
        runtime.retire_caller_pending("launcher");
        runtime
            .ensure_current("launcher", prepared.handle())
            .unwrap();
        runtime
            .finish_noncancellable_commit("launcher", prepared.handle())
            .unwrap();
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
    fn panicked_sync_commit_retires_the_exact_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "text.copy", "copy", json!({"text": "private"}))
            .unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<Result<(), ()>, ActionError> =
                runtime.commit_noncancellable("launcher", prepared.handle(), || {
                    panic!("synthetic sync commit panic")
                });
        }));
        assert!(panic.is_err());
        assert!(runtime
            .begin("launcher", "text.copy", "copy", json!({"text": "retry"}))
            .is_ok());
    }

    #[tokio::test]
    async fn async_noncancellable_commit_survives_waiter_drop_and_retires_the_slot() {
        let runtime = Arc::new(ActionRuntime::default());
        let prepared = runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(tokio::sync::Notify::new());
        let waiter = {
            let runtime = Arc::clone(&runtime);
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            let completed = Arc::clone(&completed);
            tokio::spawn(async move {
                runtime
                    .commit_noncancellable_async(
                        "launcher",
                        prepared.handle(),
                        move || async move {
                            entered.notify_one();
                            release.notified().await;
                            completed.notify_one();
                            Ok::<_, ()>("capture-session")
                        },
                    )
                    .await
            })
        };
        entered.notified().await;
        assert!(matches!(
            runtime.begin("launcher", "capture.start", "capture", json!({})),
            Err(ActionError::Busy)
        ));

        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.notify_one();
        completed.notified().await;

        let next = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match runtime.begin("launcher", "capture.start", "capture", json!({})) {
                    Ok(prepared) => break prepared,
                    Err(ActionError::Busy) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected action state: {error:?}"),
                }
            }
        })
        .await
        .expect("detached commit must retire its slot");
        runtime.ensure_current("launcher", next.handle()).unwrap();
    }

    #[tokio::test]
    async fn panicked_async_commit_returns_internal_error_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .unwrap();
        assert_eq!(
            runtime
                .commit_noncancellable_async::<(), (), _, _>(
                    "launcher",
                    prepared.handle(),
                    || async { panic!("synthetic worker panic") },
                )
                .await,
            Err(ActionError::WorkerFailed)
        );
        assert_eq!(ActionError::WorkerFailed.code(), "action_internal");
        assert!(runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .is_ok());
    }

    #[test]
    fn business_error_is_returned_once_and_retires_the_request_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.ocr",
                "analysis",
                json!({"sourceId": "clip-9", "sourceVersion": 0}),
            )
            .unwrap();
        assert_eq!(
            runtime
                .publish("image-viewer-owner", prepared.handle(), || {
                    Err::<(), _>("ocr_failed")
                })
                .unwrap(),
            Err("ocr_failed")
        );
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
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
