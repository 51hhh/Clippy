use super::RecoverySummary;
use crate::private_files::{
    replace_private_file, restrict_directory, restrict_file, write_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const RECORDINGS_DIRECTORY: &str = "recordings";
const MANIFEST_FILE: &str = "manifest.json";
const FORMAT: &str = "clippy-recording";
const SCHEMA_VERSION: u32 = 1;
const TIMEBASE_HZ: u64 = 1_000_000_000;
const MAX_SESSIONS: usize = 128;
const MAX_DIRECTORY_ENTRIES: usize = 1_024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SEGMENTS: usize = 3_600;
const MAX_DIMENSION: u32 = 16_384;
const MAX_DURATION_NS: u64 = 24 * 60 * 60 * TIMEBASE_HZ;
const MAX_SEGMENT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RecordingState {
    Recording,
    Finalizing,
    Interrupted,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordingSelection {
    source_id: String,
    physical_x: i32,
    physical_y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VideoSpec {
    width: u32,
    height: u32,
    target_fps_numerator: u32,
    target_fps_denominator: u32,
    encoder: String,
    container: String,
    include_cursor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SegmentManifest {
    index: u32,
    file_name: String,
    started_at_ns: u64,
    duration_ns: u64,
    frame_count: u64,
    byte_length: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinalOutputManifest {
    file_name: String,
    duration_ns: u64,
    frame_count: u64,
    byte_length: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryInfo {
    recovered_at_unix_ms: u64,
    original_segment_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first_invalid_segment: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordingManifest {
    format: String,
    schema_version: u32,
    session_id: String,
    state: RecordingState,
    created_at_unix_ms: u64,
    timebase_hz: u64,
    selection: RecordingSelection,
    video: VideoSpec,
    dropped_frames: u64,
    segments: Vec<SegmentManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output: Option<FinalOutputManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryInfo>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // PX-REC-01 会话 owner 接入后由 manager 构造。
pub(super) struct RecordingJournalConfig {
    pub session_id: String,
    pub source_id: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
    pub target_fps_numerator: u32,
    pub target_fps_denominator: u32,
    pub encoder: String,
    pub container: String,
    pub include_cursor: bool,
}

#[allow(dead_code)] // PX-REC-01 会话 owner 接入后跨编码线程持有。
pub(super) struct PendingSegment {
    index: u32,
    partial_path: PathBuf,
    preserve_for_recovery: bool,
}

#[allow(dead_code)] // PX-REC-01 的连续 VP9 mux 接入后跨编码线程持有。
pub(super) struct PendingFinalOutput {
    partial_path: PathBuf,
    preserve_for_recovery: bool,
}

impl Drop for PendingSegment {
    fn drop(&mut self) {
        if !self.preserve_for_recovery {
            let _ = fs::remove_file(&self.partial_path);
        }
    }
}

impl Drop for PendingFinalOutput {
    fn drop(&mut self) {
        if !self.preserve_for_recovery {
            let _ = fs::remove_file(&self.partial_path);
        }
    }
}

#[allow(dead_code)] // PX-REC-01 会话 owner 接入后成为运行时 journal。
pub(super) struct RecordingJournal {
    session_directory: PathBuf,
    manifest: RecordingManifest,
}

#[allow(dead_code)] // 当前先固定持久化协议；下一阶段由录屏 manager 消费。
impl RecordingJournal {
    pub fn create(app_data_dir: &Path, config: RecordingJournalConfig) -> Result<Self, String> {
        let root = app_data_dir.join(RECORDINGS_DIRECTORY);
        fs::create_dir_all(&root).map_err(|error| format!("创建录屏根目录失败: {error}"))?;
        let root_metadata =
            fs::symlink_metadata(&root).map_err(|error| format!("读取录屏根目录失败: {error}"))?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err("录屏根路径不是普通目录".to_string());
        }
        restrict_directory(&root).map_err(|error| format!("收紧录屏根目录权限失败: {error}"))?;
        if directory_entry_count(&root)? >= MAX_SESSIONS {
            return Err("录屏会话数量已达到 128 个上限".to_string());
        }

        let manifest = RecordingManifest {
            format: FORMAT.to_string(),
            schema_version: SCHEMA_VERSION,
            session_id: config.session_id,
            state: RecordingState::Recording,
            created_at_unix_ms: unix_time_ms().max(1),
            timebase_hz: TIMEBASE_HZ,
            selection: RecordingSelection {
                source_id: config.source_id,
                physical_x: config.physical_x,
                physical_y: config.physical_y,
                width: config.width,
                height: config.height,
            },
            video: VideoSpec {
                width: config.width,
                height: config.height,
                target_fps_numerator: config.target_fps_numerator,
                target_fps_denominator: config.target_fps_denominator,
                encoder: config.encoder,
                container: config.container,
                include_cursor: config.include_cursor,
            },
            dropped_frames: 0,
            segments: Vec::new(),
            final_output: None,
            recovery: None,
        };
        validate_manifest(&manifest, &manifest.session_id)?;

        let session_directory = root.join(&manifest.session_id);
        fs::create_dir(&session_directory)
            .map_err(|error| format!("创建录屏会话目录失败: {error}"))?;
        if let Err(error) = restrict_directory(&session_directory)
            .map_err(|error| format!("收紧录屏会话目录权限失败: {error}"))
            .and_then(|_| write_manifest(&session_directory, &manifest))
        {
            let _ = fs::remove_dir(&session_directory);
            return Err(error);
        }
        Ok(Self {
            session_directory,
            manifest,
        })
    }

    pub fn begin_segment(&self) -> Result<(File, PendingSegment), String> {
        if self.manifest.state != RecordingState::Recording {
            return Err("录屏会话不接受新分段".to_string());
        }
        if self.manifest.segments.len() >= MAX_SEGMENTS {
            return Err("录屏分段数量超过上限".to_string());
        }
        let index = u32::try_from(self.manifest.segments.len())
            .map_err(|_| "录屏分段序号溢出".to_string())?;
        let partial_path = self
            .session_directory
            .join(segment_partial_name(index, &self.manifest.video.container));
        let file = create_private_new_file(&partial_path)
            .map_err(|error| format!("创建录屏临时分段失败: {error}"))?;
        Ok((
            file,
            PendingSegment {
                index,
                partial_path,
                preserve_for_recovery: false,
            },
        ))
    }

    pub fn commit_segment(
        &mut self,
        mut pending: PendingSegment,
        file: File,
        duration_ns: u64,
        frame_count: u64,
        dropped_frames: u64,
    ) -> Result<PathBuf, String> {
        if self.manifest.state != RecordingState::Recording
            || pending.index as usize != self.manifest.segments.len()
        {
            return Err("录屏分段与当前会话状态不一致".to_string());
        }
        if duration_ns == 0 || frame_count == 0 {
            return Err("录屏分段必须包含非零时长和帧数".to_string());
        }
        let metadata = file
            .metadata()
            .map_err(|error| format!("读取录屏临时分段失败: {error}"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SEGMENT_BYTES {
            return Err("录屏临时分段大小无效".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("同步录屏临时分段失败: {error}"))?;
        drop(file);
        let partial_metadata = fs::symlink_metadata(&pending.partial_path)
            .map_err(|error| format!("读取录屏临时分段路径失败: {error}"))?;
        if partial_metadata.file_type().is_symlink() || !partial_metadata.is_file() {
            return Err("录屏临时分段不是普通文件".to_string());
        }
        restrict_file(&pending.partial_path)
            .map_err(|error| format!("收紧录屏临时分段权限失败: {error}"))?;
        let (byte_length, sha256) = hash_file(&pending.partial_path)?;
        if dropped_frames < self.manifest.dropped_frames {
            return Err("录屏累计丢帧数不能倒退".to_string());
        }
        let started_at_ns = self
            .manifest
            .segments
            .last()
            .map(|segment| {
                segment
                    .started_at_ns
                    .checked_add(segment.duration_ns)
                    .ok_or_else(|| "录屏分段起始时间溢出".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let file_name = segment_file_name(pending.index, &self.manifest.video.container);
        let previous_dropped_frames = self.manifest.dropped_frames;
        self.manifest.segments.push(SegmentManifest {
            index: pending.index,
            file_name: file_name.clone(),
            started_at_ns,
            duration_ns,
            frame_count,
            byte_length,
            sha256,
        });
        self.manifest.dropped_frames = dropped_frames;
        if let Err(error) = validate_manifest(&self.manifest, &self.manifest.session_id)
            .and_then(|_| write_manifest(&self.session_directory, &self.manifest))
        {
            self.manifest.segments.pop();
            self.manifest.dropped_frames = previous_dropped_frames;
            return Err(error);
        }
        pending.preserve_for_recovery = true;

        // 先原子提交清单，再提升已经 fsync 的 partial。若进程在两步之间退出，启动恢复会按清单中的
        // 长度与 SHA-256 验证 partial 后完成提升；未进入清单的 partial 永远不会冒充已提交分段。
        let destination = self.session_directory.join(file_name);
        replace_private_file(&pending.partial_path, &destination)
            .map_err(|error| format!("提交录屏分段失败: {error}"))?;
        sync_directory(&self.session_directory)
            .map_err(|error| format!("同步录屏会话目录失败: {error}"))?;
        Ok(destination)
    }

    pub fn begin_final_output(&self) -> Result<(File, PendingFinalOutput), String> {
        if self.manifest.state != RecordingState::Recording
            || self.manifest.segments.is_empty()
            || self.manifest.final_output.is_some()
        {
            return Err("录屏会话不接受最终输出".to_string());
        }
        let partial_path = self
            .session_directory
            .join(final_output_partial_name(&self.manifest.video.container));
        let file = create_private_new_file(&partial_path)
            .map_err(|error| format!("创建录屏最终输出失败: {error}"))?;
        Ok((
            file,
            PendingFinalOutput {
                partial_path,
                preserve_for_recovery: false,
            },
        ))
    }

    pub fn commit_final_output(
        &mut self,
        mut pending: PendingFinalOutput,
        file: File,
        duration_ns: u64,
        frame_count: u64,
    ) -> Result<PathBuf, String> {
        if self.manifest.state != RecordingState::Recording
            || self.manifest.segments.is_empty()
            || self.manifest.final_output.is_some()
        {
            return Err("录屏最终输出与当前会话状态不一致".to_string());
        }
        let expected_duration = self
            .manifest
            .segments
            .last()
            .and_then(|segment| segment.started_at_ns.checked_add(segment.duration_ns))
            .ok_or_else(|| "录屏最终输出时长溢出".to_string())?;
        let expected_frames = self
            .manifest
            .segments
            .iter()
            .try_fold(0_u64, |total, segment| {
                total
                    .checked_add(segment.frame_count)
                    .ok_or_else(|| "录屏最终输出帧数溢出".to_string())
            })?;
        if duration_ns != expected_duration || frame_count != expected_frames {
            return Err("录屏最终输出与已提交分段不一致".to_string());
        }
        let metadata = file
            .metadata()
            .map_err(|error| format!("读取录屏最终输出失败: {error}"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SESSION_BYTES {
            return Err("录屏最终输出大小无效".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("同步录屏最终输出失败: {error}"))?;
        drop(file);
        let partial_metadata = fs::symlink_metadata(&pending.partial_path)
            .map_err(|error| format!("读取录屏最终输出路径失败: {error}"))?;
        if partial_metadata.file_type().is_symlink() || !partial_metadata.is_file() {
            return Err("录屏最终输出不是普通文件".to_string());
        }
        restrict_file(&pending.partial_path)
            .map_err(|error| format!("收紧录屏最终输出权限失败: {error}"))?;
        let (byte_length, sha256) = hash_file_with_limit(&pending.partial_path, MAX_SESSION_BYTES)?;
        let file_name = final_output_file_name(&self.manifest.video.container);
        self.manifest.final_output = Some(FinalOutputManifest {
            file_name: file_name.clone(),
            duration_ns,
            frame_count,
            byte_length,
            sha256,
        });
        self.manifest.state = RecordingState::Finalizing;
        if let Err(error) = validate_manifest(&self.manifest, &self.manifest.session_id)
            .and_then(|_| write_manifest(&self.session_directory, &self.manifest))
        {
            self.manifest.final_output = None;
            self.manifest.state = RecordingState::Recording;
            return Err(error);
        }
        pending.preserve_for_recovery = true;

        let destination = self.session_directory.join(file_name);
        replace_private_file(&pending.partial_path, &destination)
            .map_err(|error| format!("提交录屏最终输出失败: {error}"))?;
        sync_directory(&self.session_directory)
            .map_err(|error| format!("同步录屏会话目录失败: {error}"))?;
        Ok(destination)
    }

    pub fn complete(&mut self) -> Result<(), String> {
        if self.manifest.segments.is_empty()
            || !matches!(
                self.manifest.state,
                RecordingState::Recording | RecordingState::Finalizing
            )
        {
            return Err("没有可封尾的录屏分段".to_string());
        }
        if self.manifest.final_output.is_some()
            && !verify_or_promote_final_output(&self.session_directory, &self.manifest)?
        {
            return Err("录屏最终输出尚未完整提交".to_string());
        }
        if self.manifest.state == RecordingState::Recording {
            self.manifest.state = RecordingState::Finalizing;
            if let Err(error) = write_manifest(&self.session_directory, &self.manifest) {
                self.manifest.state = RecordingState::Recording;
                return Err(error);
            }
        }
        self.manifest.state = RecordingState::Complete;
        if let Err(error) = write_manifest(&self.session_directory, &self.manifest) {
            self.manifest.state = RecordingState::Finalizing;
            return Err(error);
        }
        Ok(())
    }

    pub fn interrupt(&mut self) -> Result<(), String> {
        if self.manifest.state == RecordingState::Complete {
            return Ok(());
        }
        let previous_segments = self.manifest.segments.clone();
        let verified_prefix = verify_segment_prefix(&self.session_directory, &self.manifest)?;
        if verified_prefix == self.manifest.segments.len()
            && self.manifest.final_output.is_some()
            && verify_or_promote_final_output(&self.session_directory, &self.manifest)?
        {
            let previous_state = self.manifest.state;
            self.manifest.state = RecordingState::Complete;
            if let Err(error) = write_manifest(&self.session_directory, &self.manifest) {
                self.manifest.state = previous_state;
                return Err(error);
            }
            return Ok(());
        }
        self.manifest.segments.truncate(verified_prefix);
        let previous_final_output = self.manifest.final_output.take();
        let previous_state = self.manifest.state;
        self.manifest.state = RecordingState::Interrupted;
        if let Err(error) = write_manifest(&self.session_directory, &self.manifest) {
            self.manifest.state = previous_state;
            self.manifest.segments = previous_segments;
            self.manifest.final_output = previous_final_output;
            return Err(error);
        }
        discard_final_output_files(&self.session_directory, &self.manifest.video.container)?;
        Ok(())
    }

    pub fn session_directory(&self) -> &Path {
        &self.session_directory
    }
}

pub(super) fn recover_interrupted_sessions(app_data_dir: &Path) -> io::Result<RecoverySummary> {
    let root = app_data_dir.join(RECORDINGS_DIRECTORY);
    let root_metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(RecoverySummary::default());
        }
        Err(error) => return Err(error),
    };
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "录屏恢复目录不是普通目录",
        ));
    }
    restrict_directory(&root)?;

    let mut summary = RecoverySummary::default();
    let mut directory_entries = Vec::new();
    let mut entries = fs::read_dir(&root)?;
    for _ in 0..=MAX_DIRECTORY_ENTRIES {
        let Some(entry) = entries.next().transpose()? else {
            break;
        };
        directory_entries.push(entry);
    }
    if directory_entries.len() > MAX_DIRECTORY_ENTRIES {
        summary.rejected_sessions += directory_entries.len() - MAX_DIRECTORY_ENTRIES;
        directory_entries.truncate(MAX_DIRECTORY_ENTRIES);
        summary.session_limit_reached = true;
    }
    directory_entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            fs::symlink_metadata(entry.path())
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    if directory_entries.len() > MAX_SESSIONS {
        summary.rejected_sessions += directory_entries.len() - MAX_SESSIONS;
        directory_entries.truncate(MAX_SESSIONS);
        summary.session_limit_reached = true;
    }

    for entry in directory_entries {
        summary.scanned_sessions += 1;
        match reconcile_session(&entry.path(), &entry.file_name()) {
            Ok(Some(segments)) => {
                summary.interrupted_sessions += 1;
                summary.recoverable_segments += segments;
            }
            Ok(None) => {}
            Err(error) => {
                summary.rejected_sessions += 1;
                log::warn!("拒绝录屏恢复会话 {}: {error}", entry.path().display());
            }
        }
    }

    Ok(summary)
}

fn reconcile_session(
    path: &Path,
    directory_name: &std::ffi::OsStr,
) -> Result<Option<usize>, String> {
    let session_id = directory_name
        .to_str()
        .filter(|value| valid_identifier(value, 64))
        .ok_or_else(|| "会话目录名无效".to_string())?;
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("读取目录失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("会话路径不是普通目录".to_string());
    }
    restrict_directory(path).map_err(|error| format!("收紧目录权限失败: {error}"))?;

    let manifest_path = path.join(MANIFEST_FILE);
    let mut manifest = read_manifest(&manifest_path)?;
    validate_manifest(&manifest, session_id)?;
    if matches!(
        manifest.state,
        RecordingState::Interrupted | RecordingState::Complete
    ) {
        return Ok(None);
    }

    let original_segment_count = manifest.segments.len();
    let verified_prefix = verify_segment_prefix(path, &manifest)?;
    if verified_prefix == original_segment_count
        && manifest.final_output.is_some()
        && verify_or_promote_final_output(path, &manifest)?
    {
        manifest.state = RecordingState::Complete;
        manifest.recovery = None;
        discard_uncommitted_partial(path, &manifest)?;
        write_manifest(path, &manifest)?;
        return Ok(None);
    }
    let first_invalid_segment = (verified_prefix < original_segment_count)
        .then(|| u32::try_from(verified_prefix).expect("分段上限保证可以转为 u32"));
    manifest.segments.truncate(verified_prefix);
    manifest.final_output = None;
    manifest.state = RecordingState::Interrupted;
    manifest.recovery = Some(RecoveryInfo {
        recovered_at_unix_ms: unix_time_ms(),
        original_segment_count: u32::try_from(original_segment_count)
            .expect("分段上限保证可以转为 u32"),
        first_invalid_segment,
    });
    discard_uncommitted_partial(path, &manifest)?;
    discard_final_output_files(path, &manifest.video.container)?;
    write_manifest(path, &manifest)?;
    Ok(Some(verified_prefix))
}

fn discard_uncommitted_partial(
    session_directory: &Path,
    manifest: &RecordingManifest,
) -> Result<(), String> {
    let index =
        u32::try_from(manifest.segments.len()).map_err(|_| "未提交录屏分段序号溢出".to_string())?;
    let path = session_directory.join(segment_partial_name(index, &manifest.video.container));
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取未提交录屏分段失败: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("未提交录屏分段不是普通文件".to_string());
    }
    fs::remove_file(&path).map_err(|error| format!("删除未提交录屏分段失败: {error}"))?;
    sync_directory(session_directory).map_err(|error| format!("同步录屏恢复目录失败: {error}"))
}

fn read_manifest(path: &Path) -> Result<RecordingManifest, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("读取清单失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("清单不是普通文件".to_string());
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("清单超过 1 MiB 上限".to_string());
    }
    restrict_file(path).map_err(|error| format!("收紧清单权限失败: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("读取清单失败: {error}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("清单超过 1 MiB 上限".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("清单格式错误: {error}"))
}

fn validate_manifest(manifest: &RecordingManifest, directory_id: &str) -> Result<(), String> {
    if manifest.format != FORMAT || manifest.schema_version != SCHEMA_VERSION {
        return Err("清单格式或 schema 版本不受支持".to_string());
    }
    if manifest.session_id != directory_id || !valid_identifier(&manifest.session_id, 64) {
        return Err("清单会话身份与目录不一致".to_string());
    }
    if manifest.created_at_unix_ms == 0 || manifest.timebase_hz != TIMEBASE_HZ {
        return Err("清单时间基无效".to_string());
    }
    if !valid_opaque_id(&manifest.selection.source_id, 128)
        || manifest.selection.width == 0
        || manifest.selection.height == 0
        || manifest.selection.width != manifest.video.width
        || manifest.selection.height != manifest.video.height
        || manifest.video.width > MAX_DIMENSION
        || manifest.video.height > MAX_DIMENSION
    {
        return Err("清单选区或视频尺寸无效".to_string());
    }
    if manifest.video.target_fps_numerator == 0
        || manifest.video.target_fps_numerator > 240_000
        || manifest.video.target_fps_denominator == 0
        || manifest.video.target_fps_denominator > 1_000
        || u64::from(manifest.video.target_fps_numerator)
            > 240 * u64::from(manifest.video.target_fps_denominator)
        || !valid_identifier(&manifest.video.encoder, 64)
        || !valid_identifier(&manifest.video.container, 16)
    {
        return Err("清单编码参数无效".to_string());
    }
    if manifest.segments.len() > MAX_SEGMENTS {
        return Err("清单分段数量超过上限".to_string());
    }
    if !matches!(manifest.state, RecordingState::Interrupted) && manifest.recovery.is_some() {
        return Err("活动或完成会话不应包含恢复记录".to_string());
    }
    if let Some(recovery) = &manifest.recovery {
        let segment_count =
            u32::try_from(manifest.segments.len()).expect("分段上限保证可以转为 u32");
        if recovery.recovered_at_unix_ms == 0
            || recovery.original_segment_count < segment_count
            || recovery.original_segment_count as usize > MAX_SEGMENTS
            || recovery.first_invalid_segment
                != (segment_count < recovery.original_segment_count).then_some(segment_count)
        {
            return Err("清单恢复记录无效".to_string());
        }
    }

    let mut previous_end = 0_u64;
    let mut total_bytes = 0_u64;
    let mut total_frames = 0_u64;
    for (position, segment) in manifest.segments.iter().enumerate() {
        let expected_index = u32::try_from(position).expect("分段上限保证可以转为 u32");
        let expected_name = format!("segment-{expected_index:06}.{}", manifest.video.container);
        if segment.index != expected_index || segment.file_name != expected_name {
            return Err("清单分段序号或文件名不连续".to_string());
        }
        if segment.duration_ns == 0
            || segment.duration_ns > MAX_DURATION_NS
            || segment.frame_count == 0
            || segment.byte_length == 0
            || segment.byte_length > MAX_SEGMENT_BYTES
            || !valid_sha256(&segment.sha256)
        {
            return Err("清单分段元数据无效".to_string());
        }
        if position > 0 && segment.started_at_ns < previous_end {
            return Err("清单分段时间线重叠".to_string());
        }
        previous_end = segment
            .started_at_ns
            .checked_add(segment.duration_ns)
            .ok_or_else(|| "清单分段时间线溢出".to_string())?;
        if previous_end > MAX_DURATION_NS {
            return Err("清单会话时长超过上限".to_string());
        }
        total_bytes = total_bytes
            .checked_add(segment.byte_length)
            .ok_or_else(|| "清单分段大小溢出".to_string())?;
        total_frames = total_frames
            .checked_add(segment.frame_count)
            .ok_or_else(|| "清单分段帧数溢出".to_string())?;
        if total_bytes > MAX_SESSION_BYTES {
            return Err("清单会话大小超过恢复上限".to_string());
        }
    }
    if let Some(output) = &manifest.final_output {
        if output.file_name != final_output_file_name(&manifest.video.container)
            || output.duration_ns == 0
            || output.duration_ns != previous_end
            || output.frame_count == 0
            || output.frame_count != total_frames
            || output.byte_length == 0
            || output.byte_length > MAX_SESSION_BYTES
            || !valid_sha256(&output.sha256)
            || !matches!(
                manifest.state,
                RecordingState::Finalizing | RecordingState::Complete
            )
        {
            return Err("清单最终输出元数据无效".to_string());
        }
    }
    Ok(())
}

fn verify_segment_prefix(path: &Path, manifest: &RecordingManifest) -> Result<usize, String> {
    for (position, segment) in manifest.segments.iter().enumerate() {
        let segment_path = path.join(&segment.file_name);
        let metadata = match fs::symlink_metadata(&segment_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if promote_committed_partial(path, manifest, segment)? {
                    fs::symlink_metadata(&segment_path)
                        .map_err(|error| format!("读取已提升分段失败: {error}"))?
                } else {
                    return Ok(position);
                }
            }
            Err(error) => return Err(format!("读取分段失败: {error}")),
        };
        if metadata.file_type().is_symlink() {
            return Err("分段不能是符号链接".to_string());
        }
        if !metadata.is_file() || metadata.len() != segment.byte_length {
            return Ok(position);
        }
        restrict_file(&segment_path).map_err(|error| format!("收紧分段权限失败: {error}"))?;
        let (byte_length, sha256) = hash_file(&segment_path)?;
        if byte_length != segment.byte_length || sha256 != segment.sha256 {
            return Ok(position);
        }
    }
    Ok(manifest.segments.len())
}

fn verify_or_promote_final_output(
    session_directory: &Path,
    manifest: &RecordingManifest,
) -> Result<bool, String> {
    let Some(output) = &manifest.final_output else {
        return Ok(false);
    };
    let destination = session_directory.join(&output.file_name);
    let metadata = match fs::symlink_metadata(&destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let partial =
                session_directory.join(final_output_partial_name(&manifest.video.container));
            let partial_metadata = match fs::symlink_metadata(&partial) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(format!("读取待恢复最终输出失败: {error}")),
            };
            if partial_metadata.file_type().is_symlink() {
                return Err("录屏最终输出不能是符号链接".to_string());
            }
            if !partial_metadata.is_file() || partial_metadata.len() != output.byte_length {
                return Ok(false);
            }
            restrict_file(&partial).map_err(|error| format!("收紧最终输出权限失败: {error}"))?;
            let (byte_length, sha256) = hash_file_with_limit(&partial, MAX_SESSION_BYTES)?;
            if byte_length != output.byte_length || sha256 != output.sha256 {
                return Ok(false);
            }
            replace_private_file(&partial, &destination)
                .map_err(|error| format!("提升已提交最终输出失败: {error}"))?;
            sync_directory(session_directory)
                .map_err(|error| format!("同步最终输出恢复目录失败: {error}"))?;
            fs::symlink_metadata(&destination)
                .map_err(|error| format!("读取已提升最终输出失败: {error}"))?
        }
        Err(error) => return Err(format!("读取最终输出失败: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err("录屏最终输出不能是符号链接".to_string());
    }
    if !metadata.is_file() || metadata.len() != output.byte_length {
        return Ok(false);
    }
    restrict_file(&destination).map_err(|error| format!("收紧最终输出权限失败: {error}"))?;
    let (byte_length, sha256) = hash_file_with_limit(&destination, MAX_SESSION_BYTES)?;
    Ok(byte_length == output.byte_length && sha256 == output.sha256)
}

fn discard_final_output_files(session_directory: &Path, container: &str) -> Result<(), String> {
    let mut removed = false;
    for file_name in [
        final_output_partial_name(container),
        final_output_file_name(container),
    ] {
        let path = session_directory.join(file_name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("读取废弃最终输出失败: {error}")),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("废弃最终输出不是普通文件".to_string());
        }
        fs::remove_file(&path).map_err(|error| format!("删除废弃最终输出失败: {error}"))?;
        removed = true;
    }
    if removed {
        sync_directory(session_directory)
            .map_err(|error| format!("同步最终输出清理目录失败: {error}"))?;
    }
    Ok(())
}

fn promote_committed_partial(
    session_directory: &Path,
    manifest: &RecordingManifest,
    segment: &SegmentManifest,
) -> Result<bool, String> {
    let partial = session_directory.join(segment_partial_name(
        segment.index,
        &manifest.video.container,
    ));
    let metadata = match fs::symlink_metadata(&partial) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("读取待恢复临时分段失败: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err("录屏临时分段不能是符号链接".to_string());
    }
    if !metadata.is_file() || metadata.len() != segment.byte_length {
        return Ok(false);
    }
    restrict_file(&partial).map_err(|error| format!("收紧临时分段权限失败: {error}"))?;
    let (byte_length, sha256) = hash_file(&partial)?;
    if byte_length != segment.byte_length || sha256 != segment.sha256 {
        return Ok(false);
    }
    let destination = session_directory.join(&segment.file_name);
    replace_private_file(&partial, &destination)
        .map_err(|error| format!("提升已提交临时分段失败: {error}"))?;
    sync_directory(session_directory).map_err(|error| format!("同步恢复目录失败: {error}"))?;
    Ok(true)
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    hash_file_with_limit(path, MAX_SEGMENT_BYTES)
}

fn hash_file_with_limit(path: &Path, byte_limit: u64) -> Result<(u64, String), String> {
    let file = File::open(path).map_err(|error| format!("打开分段失败: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| format!("读取分段元数据失败: {error}"))?
        .is_file()
    {
        return Err("分段不是普通文件".to_string());
    }
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut byte_length = 0_u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("读取分段失败: {error}"))?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(read as u64)
            .ok_or_else(|| "分段大小溢出".to_string())?;
        if byte_length > byte_limit {
            return Err("录屏文件超过恢复大小上限".to_string());
        }
        hasher.update(&buffer[..read]);
    }
    Ok((byte_length, format!("{:x}", hasher.finalize())))
}

#[allow(dead_code)]
fn directory_entry_count(path: &Path) -> Result<usize, String> {
    let mut count = 0_usize;
    let mut entries = fs::read_dir(path).map_err(|error| format!("读取录屏根目录失败: {error}"))?;
    while count <= MAX_SESSIONS {
        let Some(_) = entries
            .next()
            .transpose()
            .map_err(|error| format!("读取录屏根目录项失败: {error}"))?
        else {
            break;
        };
        count += 1;
    }
    Ok(count)
}

#[allow(dead_code)]
fn create_private_new_file(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    restrict_file(path)?;
    Ok(file)
}

#[allow(dead_code)]
fn segment_file_name(index: u32, container: &str) -> String {
    format!("segment-{index:06}.{container}")
}

fn segment_partial_name(index: u32, container: &str) -> String {
    format!(".segment-{index:06}.{container}.partial")
}

fn final_output_file_name(container: &str) -> String {
    format!("recording.{container}")
}

fn final_output_partial_name(container: &str) -> String {
    format!(".recording.{container}.partial")
}

fn write_manifest(session_directory: &Path, manifest: &RecordingManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("序列化恢复清单失败: {error}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("恢复清单超过 1 MiB 上限".to_string());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = session_directory.join(format!(
        ".manifest-recovery-{}-{nonce}.tmp",
        std::process::id()
    ));
    let destination = session_directory.join(MANIFEST_FILE);
    let result = write_private(&temporary, &bytes)
        .and_then(|_| replace_private_file(&temporary, &destination))
        .and_then(|_| sync_directory(session_directory));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| format!("原子写入恢复清单失败: {error}"))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn valid_identifier(value: &str, maximum_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}

fn valid_opaque_id(value: &str, maximum_length: usize) -> bool {
    !value.is_empty() && value.len() <= maximum_length && !value.chars().any(char::is_control)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::io::Write as _;

    fn fixture_manifest(session_id: &str, state: RecordingState) -> RecordingManifest {
        RecordingManifest {
            format: FORMAT.to_string(),
            schema_version: SCHEMA_VERSION,
            session_id: session_id.to_string(),
            state,
            created_at_unix_ms: 1_700_000_000_000,
            timebase_hz: TIMEBASE_HZ,
            selection: RecordingSelection {
                source_id: "display-1".to_string(),
                physical_x: 10,
                physical_y: 20,
                width: 640,
                height: 480,
            },
            video: VideoSpec {
                width: 640,
                height: 480,
                target_fps_numerator: 30,
                target_fps_denominator: 1,
                encoder: "fixture".to_string(),
                container: "webm".to_string(),
                include_cursor: true,
            },
            dropped_frames: 0,
            segments: Vec::new(),
            final_output: None,
            recovery: None,
        }
    }

    fn write_manifest_fixture(directory: &Path, manifest: &RecordingManifest) {
        fs::write(
            directory.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(manifest).unwrap(),
        )
        .unwrap();
    }

    fn add_segment(directory: &Path, manifest: &mut RecordingManifest, bytes: &[u8]) {
        let index = manifest.segments.len() as u32;
        let file_name = format!("segment-{index:06}.{}", manifest.video.container);
        fs::write(directory.join(&file_name), bytes).unwrap();
        manifest.segments.push(SegmentManifest {
            index,
            file_name,
            started_at_ns: u64::from(index) * TIMEBASE_HZ,
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            byte_length: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
    }

    fn create_session(root: &Path, session_id: &str) -> std::path::PathBuf {
        let recordings = root.join(RECORDINGS_DIRECTORY);
        fs::create_dir_all(&recordings).unwrap();
        let session = recordings.join(session_id);
        fs::create_dir(&session).unwrap();
        session
    }

    fn journal_config(session_id: &str) -> RecordingJournalConfig {
        RecordingJournalConfig {
            session_id: session_id.to_string(),
            source_id: "monitor-7".to_string(),
            physical_x: -1920,
            physical_y: 0,
            width: 640,
            height: 480,
            target_fps_numerator: 30,
            target_fps_denominator: 1,
            encoder: "mjpeg-diagnostic".to_string(),
            container: "avi".to_string(),
            include_cursor: true,
        }
    }

    #[test]
    fn journal_commits_private_segment_and_completes_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), journal_config("journal-1")).unwrap();
        let session = journal.session_directory().to_path_buf();
        let (mut file, pending) = journal.begin_segment().unwrap();
        file.write_all(b"seekable avi fixture").unwrap();
        let segment = journal
            .commit_segment(pending, file, TIMEBASE_HZ, 30, 2)
            .unwrap();
        let (mut final_file, pending_final) = journal.begin_final_output().unwrap();
        final_file.write_all(b"single playable avi").unwrap();
        let final_output = journal
            .commit_final_output(pending_final, final_file, TIMEBASE_HZ, 30)
            .unwrap();
        journal.complete().unwrap();

        assert_eq!(fs::read(&segment).unwrap(), b"seekable avi fixture");
        assert_eq!(fs::read(&final_output).unwrap(), b"single playable avi");
        assert!(!session.join(".segment-000000.avi.partial").exists());
        assert!(!session.join(".recording.avi.partial").exists());
        assert!(crate::private_files::is_private(&segment));
        assert!(crate::private_files::is_private(&final_output));
        assert!(crate::private_files::is_private(
            &session.join(MANIFEST_FILE)
        ));
        let manifest = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.state, RecordingState::Complete);
        assert_eq!(manifest.dropped_frames, 2);
        assert_eq!(manifest.segments.len(), 1);
        assert_eq!(manifest.segments[0].frame_count, 30);
        assert_eq!(manifest.segments[0].byte_length, 20);
        let output = manifest.final_output.unwrap();
        assert_eq!(output.file_name, "recording.avi");
        assert_eq!(output.duration_ns, TIMEBASE_HZ);
        assert_eq!(output.frame_count, 30);
        assert_eq!(output.byte_length, 19);
    }

    #[test]
    fn recovery_promotes_manifest_committed_final_output_and_completes_session() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "promote-final");
        let segment_bytes = b"recoverable segment";
        let final_bytes = b"recoverable final output";
        let mut manifest = fixture_manifest("promote-final", RecordingState::Finalizing);
        add_segment(&session, &mut manifest, segment_bytes);
        manifest.final_output = Some(FinalOutputManifest {
            file_name: "recording.webm".to_string(),
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            byte_length: final_bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(final_bytes)),
        });
        fs::write(session.join(".recording.webm.partial"), final_bytes).unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.interrupted_sessions, 0);
        assert_eq!(summary.recoverable_segments, 0);
        assert_eq!(
            fs::read(session.join("recording.webm")).unwrap(),
            final_bytes
        );
        assert!(!session.join(".recording.webm.partial").exists());
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Complete);
        assert!(recovered.final_output.is_some());
    }

    #[test]
    fn damaged_final_output_falls_back_to_recoverable_segments() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "damaged-final");
        let segment_bytes = b"recoverable segment";
        let expected_final = b"expected final output";
        let mut manifest = fixture_manifest("damaged-final", RecordingState::Finalizing);
        add_segment(&session, &mut manifest, segment_bytes);
        manifest.final_output = Some(FinalOutputManifest {
            file_name: "recording.webm".to_string(),
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            byte_length: expected_final.len() as u64,
            sha256: format!("{:x}", Sha256::digest(expected_final)),
        });
        fs::write(session.join("recording.webm"), b"tampered final output").unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 1);
        assert!(!session.join("recording.webm").exists());
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert_eq!(recovered.segments.len(), 1);
        assert!(recovered.final_output.is_none());
    }

    #[test]
    fn invalid_final_output_metadata_removes_uncommitted_partial() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), journal_config("invalid-final")).unwrap();
        let session = journal.session_directory().to_path_buf();
        let (mut segment_file, pending_segment) = journal.begin_segment().unwrap();
        segment_file.write_all(b"segment").unwrap();
        journal
            .commit_segment(pending_segment, segment_file, TIMEBASE_HZ, 30, 0)
            .unwrap();
        let (mut final_file, pending_final) = journal.begin_final_output().unwrap();
        final_file.write_all(b"final").unwrap();
        assert!(journal
            .commit_final_output(pending_final, final_file, TIMEBASE_HZ - 1, 30)
            .is_err());
        assert!(!session.join(".recording.avi.partial").exists());
        let manifest = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.state, RecordingState::Recording);
        assert!(manifest.final_output.is_none());
    }

    #[test]
    fn recovery_promotes_manifest_committed_partial_before_verifying_prefix() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "promote-partial");
        let bytes = b"committed before rename";
        let mut manifest = fixture_manifest("promote-partial", RecordingState::Recording);
        manifest.video.container = "avi".to_string();
        manifest.segments.push(SegmentManifest {
            index: 0,
            file_name: "segment-000000.avi".to_string(),
            started_at_ns: 0,
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            byte_length: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
        fs::write(session.join(".segment-000000.avi.partial"), bytes).unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 1);
        assert_eq!(fs::read(session.join("segment-000000.avi")).unwrap(), bytes);
        assert!(!session.join(".segment-000000.avi.partial").exists());
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert_eq!(recovered.segments.len(), 1);
    }

    #[test]
    fn invalid_segment_metadata_never_enters_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), journal_config("journal-invalid")).unwrap();
        let manifest_path = journal.session_directory().join(MANIFEST_FILE);
        let (mut file, pending) = journal.begin_segment().unwrap();
        file.write_all(b"bytes").unwrap();
        assert!(journal.commit_segment(pending, file, 0, 1, 0).is_err());
        let manifest = read_manifest(&manifest_path).unwrap();
        assert!(manifest.segments.is_empty());
        assert_eq!(manifest.state, RecordingState::Recording);
        assert!(!journal
            .session_directory()
            .join(".segment-000000.avi.partial")
            .exists());
        assert!(journal.complete().is_err());
        journal.interrupt().unwrap();
        assert_eq!(
            read_manifest(&manifest_path).unwrap().state,
            RecordingState::Interrupted
        );
    }

    #[test]
    fn abandoned_pending_segment_removes_its_temporary_file() {
        let temporary = tempfile::tempdir().unwrap();
        let journal =
            RecordingJournal::create(temporary.path(), journal_config("journal-abandoned"))
                .unwrap();
        let path = journal
            .session_directory()
            .join(".segment-000000.avi.partial");
        let (file, pending) = journal.begin_segment().unwrap();
        assert!(path.exists());
        drop(file);
        drop(pending);
        assert!(!path.exists());
    }

    #[test]
    fn invalid_journal_identity_is_rejected_before_session_creation() {
        let temporary = tempfile::tempdir().unwrap();
        let mut config = journal_config("../escape");
        assert!(RecordingJournal::create(temporary.path(), config.clone()).is_err());
        config.session_id = "valid".to_string();
        config.container = "../avi".to_string();
        assert!(RecordingJournal::create(temporary.path(), config).is_err());
        let root = temporary.path().join(RECORDINGS_DIRECTORY);
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }

    #[test]
    fn active_session_becomes_interrupted_with_verified_segments() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-1");
        let mut manifest = fixture_manifest("session-1", RecordingState::Recording);
        add_segment(&session, &mut manifest, b"segment zero");
        add_segment(&session, &mut manifest, b"segment one");
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.scanned_sessions, 1);
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 2);
        assert_eq!(summary.rejected_sessions, 0);

        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert_eq!(recovered.segments.len(), 2);
        let recovery = recovered.recovery.unwrap();
        assert_eq!(recovery.original_segment_count, 2);
        assert_eq!(recovery.first_invalid_segment, None);
    }

    #[test]
    fn damaged_tail_is_truncated_without_discarding_the_prefix() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-2");
        let mut manifest = fixture_manifest("session-2", RecordingState::Finalizing);
        add_segment(&session, &mut manifest, b"valid prefix");
        add_segment(&session, &mut manifest, b"damaged tail");
        fs::write(session.join("segment-000001.webm"), b"tamperedtail").unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.recoverable_segments, 1);
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert_eq!(recovered.segments.len(), 1);
        assert_eq!(recovered.recovery.unwrap().first_invalid_segment, Some(1));
    }

    #[test]
    fn missing_tail_is_truncated_without_discarding_the_prefix() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-missing");
        let mut manifest = fixture_manifest("session-missing", RecordingState::Recording);
        add_segment(&session, &mut manifest, b"valid prefix");
        add_segment(&session, &mut manifest, b"missing tail");
        fs::remove_file(session.join("segment-000001.webm")).unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.recoverable_segments, 1);
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.segments.len(), 1);
        assert_eq!(recovered.recovery.unwrap().first_invalid_segment, Some(1));
    }

    #[test]
    fn traversal_or_unknown_manifest_fields_are_rejected() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-3");
        let mut manifest = fixture_manifest("session-3", RecordingState::Recording);
        add_segment(&session, &mut manifest, b"segment");
        manifest.segments[0].file_name = "../outside.webm".to_string();
        let mut value = serde_json::to_value(manifest).unwrap();
        value["unexpected"] = serde_json::json!(true);
        fs::write(
            session.join(MANIFEST_FILE),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.rejected_sessions, 1);
        assert_eq!(summary.interrupted_sessions, 0);
    }

    #[test]
    fn oversized_manifest_is_rejected_before_json_parsing() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-4");
        fs::write(
            session.join(MANIFEST_FILE),
            vec![b' '; MAX_MANIFEST_BYTES as usize + 1],
        )
        .unwrap();

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.rejected_sessions, 1);
    }

    #[test]
    fn complete_session_is_not_rewritten_or_rehashed() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "session-5");
        let mut manifest = fixture_manifest("session-5", RecordingState::Complete);
        add_segment(&session, &mut manifest, b"completed");
        write_manifest_fixture(&session, &manifest);
        fs::remove_file(session.join("segment-000000.webm")).unwrap();

        let before = fs::read(session.join(MANIFEST_FILE)).unwrap();
        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        let after = fs::read(session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(before, after);
        assert_eq!(summary.interrupted_sessions, 0);
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_session_is_rejected() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let recordings = temporary.path().join(RECORDINGS_DIRECTORY);
        let outside = temporary.path().join("outside");
        fs::create_dir(&recordings).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, recordings.join("session-link")).unwrap();

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.rejected_sessions, 1);
    }

    #[test]
    fn invalid_directory_identity_is_rejected_before_opening_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let recordings = temporary.path().join(RECORDINGS_DIRECTORY);
        fs::create_dir(&recordings).unwrap();
        fs::create_dir(recordings.join("..invalid..".repeat(20))).unwrap();

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.rejected_sessions, 1);
        assert!(reconcile_session(temporary.path(), OsStr::new("../escape")).is_err());
    }
}
