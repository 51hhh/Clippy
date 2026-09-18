use super::model::*;
use crate::code_detection::CodeScanResponse;
use crate::translation::types::{ServiceTranslation, TranslationBatch};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};

const ACTIVE: u8 = 1;
const READY: u8 = 2;
const DESTROYED: u8 = 4;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum Channel {
    Ocr,
    Scan,
    Translation,
    Color,
    Output,
    Text,
}
#[derive(Default)]
struct SessionState {
    sensitive: bool,
    latest: HashMap<Channel, u64>,
    ocr: Option<crate::ocr::StructuredOcr>,
    scan: Option<CodeScanResponse>,
    translation: Option<TranslationBatch>,
    color: Option<ViewerColor>,
}
pub(super) struct ViewerSession {
    pub payload: ViewerPayload,
    /// 当前历史条目的扁平预览，供 OCR、扫码、取色以及输出身份使用。
    pub png: Arc<Vec<u8>>,
    /// 画布原图。内部修订指向唯一根图；普通图片与 `png` 共享同一 Arc。
    pub source_png: Arc<Vec<u8>>,
    /// 只有普通历史图片能在首次保存时迁移自身 BLOB 为根资产。
    pub root_clip_id: Option<i64>,
    base_project: Option<crate::pin::commands::PinCanvasProject>,
    pub translation: Arc<crate::translation::TranslationService>,
    state: Mutex<SessionState>,
    // 原生事件线程不能等待输出锁：Pin builder/剪贴板可能反过来等待该线程。
    lifecycle: AtomicU8,
    pin_uncertain: AtomicBool,
}
impl ViewerSession {
    pub fn new(
        clip_id: i64,
        hash: String,
        sensitive: bool,
        png: Vec<u8>,
        dimensions: (u32, u32),
        managed: Option<(
            Vec<u8>,
            crate::pin::commands::PinCanvasProject,
            serde_json::Value,
        )>,
    ) -> Self {
        let id = crate::image_io::unique_image_id();
        let payload = ViewerPayload {
            handle: ViewerHandle {
                session_id: id.clone(),
                snapshot_id: format!("snapshot-{id}"),
            },
            label: format!("image-viewer-{id}"),
            source: ViewerSource {
                clip_id: Some(clip_id),
                content_hash: hash,
                width: dimensions.0,
                height: dimensions.1,
                byte_length: png.len(),
                media_type: "image/png",
                sensitive,
            },
            initial_project: managed.as_ref().map(|(_, _, initial)| initial.clone()),
            limits: ViewerLimits {
                can_edit: true,
                can_scan: true,
                reason: None,
            },
        };
        let png = Arc::new(png);
        let (source_png, root_clip_id, base_project) = match managed {
            Some((source, project, _)) => (Arc::new(source), None, Some(project)),
            None => (Arc::clone(&png), Some(clip_id), None),
        };
        Self {
            payload,
            png,
            source_png,
            root_clip_id,
            base_project,
            translation: Arc::new(crate::translation::TranslationService::new()),
            lifecycle: AtomicU8::new(ACTIVE),
            pin_uncertain: AtomicBool::new(false),
            state: Mutex::new(SessionState {
                sensitive,
                ..Default::default()
            }),
        }
    }

    pub fn effective_project(
        &self,
        submitted: Option<&crate::pin::commands::PinCanvasProject>,
    ) -> Option<crate::pin::commands::PinCanvasProject> {
        submitted.cloned().or_else(|| self.base_project.clone())
    }
    pub fn authorize(&self, caller: &str, handle: &ViewerHandle) -> Result<(), ViewerError> {
        if caller != self.payload.label || handle != &self.payload.handle {
            return Err("forbidden".into());
        }
        Ok(())
    }
    pub fn payload(&self) -> Result<ViewerPayload, ViewerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        if !self.is_active() {
            return Err("closed".into());
        }
        let mut payload = self.payload.clone();
        payload.source.sensitive = state.sensitive;
        Ok(payload)
    }
    pub fn mark_ready(&self) -> Result<(), ViewerError> {
        self.lifecycle
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & ACTIVE != 0).then_some(state | READY)
            })
            .map(|_| ())
            .map_err(|_| ViewerError::new("closed"))
    }
    pub fn is_active(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) & ACTIVE != 0
    }
    pub fn ready(&self) -> bool {
        self.lifecycle.load(Ordering::Acquire) & (ACTIVE | READY) == ACTIVE | READY
    }
    pub fn begin(&self, channel: Channel, request: &ViewerRequest) -> Result<(), ViewerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        if !self.is_active() {
            return Err("closed".into());
        }
        // 前端 JS 可精确表达的正整数；同请求的重复发送不重做副作用。
        if request.request_id == 0
            || request.request_id > 9_007_199_254_740_991
            || state
                .latest
                .get(&channel)
                .is_some_and(|id| *id >= request.request_id)
        {
            return Err("stale_request".into());
        }
        state.latest.insert(channel, request.request_id);
        match channel {
            Channel::Ocr => state.ocr = None,
            Channel::Scan => state.scan = None,
            Channel::Translation => state.translation = None,
            Channel::Color => state.color = None,
            _ => {}
        }
        Ok(())
    }
    // 结果发布、剪贴板/文件提交与关闭共用此锁，耗时推理/渲染在进入前完成。
    pub fn commit<T>(
        &self,
        channel: Channel,
        request: &ViewerRequest,
        work: impl FnOnce() -> Result<T, ViewerError>,
    ) -> Result<T, ViewerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&state, channel, request)?;
        work()
    }
    fn check_current(
        &self,
        state: &SessionState,
        channel: Channel,
        request: &ViewerRequest,
    ) -> Result<(), ViewerError> {
        if !self.is_active() {
            return Err("closed".into());
        }
        if state.latest.get(&channel) != Some(&request.request_id) {
            return Err("stale_request".into());
        }
        Ok(())
    }
    pub fn publish_ocr(
        &self,
        channel: Channel,
        request: &ViewerRequest,
        result: crate::ocr::StructuredOcr,
    ) -> Result<(), ViewerError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&s, channel, request)?;
        s.ocr = Some(result);
        Ok(())
    }
    pub fn ocr(&self) -> Option<crate::ocr::StructuredOcr> {
        let state = self.state.lock().ok()?;
        self.is_active().then(|| state.ocr.clone()).flatten()
    }
    pub fn publish_scan(
        &self,
        request: &ViewerRequest,
        result: CodeScanResponse,
    ) -> Result<(), ViewerError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&s, Channel::Scan, request)?;
        s.scan = Some(result);
        Ok(())
    }
    pub fn publish_translation(
        &self,
        request: &ViewerRequest,
        result: TranslationBatch,
    ) -> Result<(), ViewerError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&s, Channel::Translation, request)?;
        s.translation = Some(result);
        Ok(())
    }
    pub fn publish_color(
        &self,
        request: &ViewerRequest,
        result: ViewerColor,
    ) -> Result<(), ViewerError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&s, Channel::Color, request)?;
        s.color = Some(result);
        Ok(())
    }
    pub fn copy_text<T>(
        &self,
        request: &ViewerRequest,
        source: TextSource,
        index: usize,
        write: impl FnOnce(&str) -> Result<T, ViewerError>,
    ) -> Result<T, ViewerError> {
        let s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        self.check_current(&s, Channel::Text, request)?;
        let text = match source {
            TextSource::Ocr if index == 0 => s.ocr.as_ref().map(|v| v.text.as_str()),
            TextSource::Code => s
                .scan
                .as_ref()
                .and_then(|v| v.results.get(index))
                .map(|v| v.text.as_str()),
            TextSource::Translation => s
                .translation
                .as_ref()
                .and_then(|v| v.services.get(index))
                .and_then(|v| match v {
                    ServiceTranslation::Ok {
                        translated_text, ..
                    } => Some(translated_text.as_str()),
                    _ => None,
                }),
            TextSource::Color if index == 0 => s.color.as_ref().map(|v| v.hex.as_str()),
            _ => None,
        }
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ViewerError::new("empty_input"))?;
        write(text)
    }
    pub fn protect_sensitive(&self, currently_sensitive: bool) -> Result<(), ViewerError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        if !self.is_active() {
            return Err("closed".into());
        }
        s.sensitive |= currently_sensitive;
        if s.sensitive {
            Err("sensitive_content".into())
        } else {
            Ok(())
        }
    }
    /// 原生销毁只撤销身份；不等待正在调用原生 API 的输出工作线程。
    pub fn invalidate(&self) {
        self.lifecycle
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                Some((state & !ACTIVE) | DESTROYED)
            })
            .ok();
        self.translation.next_request_id();
    }
    /// 显式关闭先撤销后续任务，再在工作线程等已入场的最终输出完成。
    pub fn deactivate(&self) {
        self.lifecycle.fetch_and(!ACTIVE, Ordering::AcqRel);
        self.translation.next_request_id();
        drop(self.state.lock());
    }
    pub fn restore_after_close_failure(&self) {
        self.lifecycle
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state & DESTROYED == 0).then_some(state | ACTIVE)
            })
            .ok();
    }
    pub fn expire_unready(&self) -> bool {
        self.lifecycle
            .compare_exchange(ACTIVE, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn commit_pin<T>(
        &self,
        request: &ViewerRequest,
        work: impl FnOnce() -> Result<T, crate::pin::commands::ScreenshotPinCreateError>,
    ) -> Result<T, ViewerError> {
        self.commit(Channel::Output, request, || {
            if self.pin_uncertain.load(Ordering::Acquire) {
                return Err("pin_creation_uncertain".into());
            }
            work().map_err(|error| {
                if error.is_uncertain() {
                    self.pin_uncertain.store(true, Ordering::Release);
                    ViewerError::new("pin_creation_uncertain")
                } else {
                    ViewerError::new("pin_creation_failed")
                }
            })
        })
    }
}
#[derive(Default)]
pub struct ViewerManager {
    entries: Mutex<HashMap<String, Arc<ViewerSession>>>,
    // 弱引用追踪真正PNG存活期；关闭窗口但OCR仍持Arc时继续计费。
    sources: Mutex<Vec<(Weak<Vec<u8>>, usize)>>,
}
impl ViewerManager {
    pub(super) fn remaining_source_budget(&self) -> Result<usize, ViewerError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        if entries.len() >= MAX_WINDOWS {
            return Err("image_too_large".into());
        }
        let mut sources = self
            .sources
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        Ok(MAX_TOTAL_BYTES.saturating_sub(retained_source_bytes(&mut sources)))
    }
    pub(super) fn find(&self, id: i64, hash: &str) -> Option<Arc<ViewerSession>> {
        self.entries
            .lock()
            .ok()?
            .values()
            .find(|v| {
                v.payload.source.clip_id == Some(id)
                    && v.payload.source.content_hash == hash
                    && v.is_active()
            })
            .cloned()
    }
    pub(super) fn insert(&self, entry: Arc<ViewerSession>) -> Result<(), ViewerError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        let mut sources = self
            .sources
            .lock()
            .map_err(|_| ViewerError::new("internal"))?;
        let distinct_source_bytes = if Arc::ptr_eq(&entry.png, &entry.source_png) {
            0
        } else {
            entry.source_png.len()
        };
        check_budget(
            entries.len(),
            retained_source_bytes(&mut sources),
            entry.png.len().saturating_add(distinct_source_bytes),
        )?;
        if entries.contains_key(&entry.payload.label) {
            return Err("busy".into());
        }
        sources.push((Arc::downgrade(&entry.png), entry.png.len()));
        if !Arc::ptr_eq(&entry.png, &entry.source_png) {
            sources.push((Arc::downgrade(&entry.source_png), entry.source_png.len()));
        }
        entries.insert(entry.payload.label.clone(), entry);
        Ok(())
    }
    pub(super) fn get(&self, label: &str) -> Result<Arc<ViewerSession>, ViewerError> {
        if !is_viewer_label(label) {
            return Err("forbidden".into());
        }
        self.entries
            .lock()
            .map_err(|_| ViewerError::new("internal"))?
            .get(label)
            .cloned()
            .ok_or_else(|| "not_found".into())
    }
    pub fn remove(&self, label: &str) {
        let entry = self
            .entries
            .lock()
            .ok()
            .and_then(|mut entries| entries.remove(label));
        if let Some(entry) = entry {
            entry.invalidate();
        }
    }
    pub fn is_ready(&self, label: &str) -> bool {
        self.get(label).is_ok_and(|s| s.ready())
    }
}

pub(super) fn check_budget(windows: usize, bytes: usize, added: usize) -> Result<(), ViewerError> {
    if windows >= MAX_WINDOWS || bytes.saturating_add(added) > MAX_TOTAL_BYTES {
        Err("image_too_large".into())
    } else {
        Ok(())
    }
}

fn retained_source_bytes(sources: &mut Vec<(Weak<Vec<u8>>, usize)>) -> usize {
    sources.retain(|(source, _)| source.strong_count() > 0);
    sources.iter().map(|(_, bytes)| *bytes).sum()
}
