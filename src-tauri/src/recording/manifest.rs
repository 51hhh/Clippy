use super::RecoverySummary;
use crate::private_files::{
    replace_private_file, restrict_directory, restrict_file, write_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const RECORDINGS_DIRECTORY: &str = "recordings";
const MANIFEST_FILE: &str = "manifest.json";
const FORMAT: &str = "clippy-recording";
const SCHEMA_VERSION: u32 = 1;
const AV_SCHEMA_VERSION: u32 = 2;
const TIMEBASE_HZ: u64 = 1_000_000_000;
const OPUS_SAMPLE_RATE_HZ: u32 = 48_000;
const OPUS_SEEK_PRE_ROLL_NS: u64 = 80_000_000;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioSpec {
    sample_rate_hz: u32,
    channels: u16,
    encoder: String,
    pre_skip_frames: u16,
    codec_delay_ns: u64,
    seek_pre_roll_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioArtifactStats {
    packet_count: u64,
    pcm_frame_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SegmentManifest {
    index: u32,
    file_name: String,
    started_at_ns: u64,
    duration_ns: u64,
    frame_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<AudioArtifactStats>,
    byte_length: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinalOutputManifest {
    file_name: String,
    duration_ns: u64,
    frame_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<AudioArtifactStats>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<AudioSpec>,
    dropped_frames: u64,
    segments: Vec<SegmentManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    final_output: Option<FinalOutputManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoveryInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingLibraryArtifact {
    pub artifact_id: String,
    pub display_name: String,
    pub duration_ms: u64,
    pub frame_count: u64,
    pub byte_length: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingLibraryItem {
    pub session_id: String,
    pub state: &'static str,
    pub created_at_unix_ms: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps_numerator: u32,
    pub target_fps_denominator: u32,
    pub encoder: String,
    pub container: String,
    pub include_cursor: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<RecordingLibraryAudio>,
    pub dropped_frames: u64,
    pub duration_ms: u64,
    pub frame_count: u64,
    pub byte_length: u64,
    pub can_merge: bool,
    pub can_thumbnail: bool,
    pub artifacts: Vec<RecordingLibraryArtifact>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingLibraryAudio {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub encoder: String,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedRecordingArtifact {
    pub path: PathBuf,
    pub suggested_file_name: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[cfg(feature = "recording-vp9-prototype")]
#[derive(Debug, Clone)]
pub(super) struct RecordingThumbnailSource {
    pub artifact: ResolvedRecordingArtifact,
    pub width: u32,
    pub height: u32,
    pub target_fps_numerator: u32,
    pub target_fps_denominator: u32,
    pub duration_ns: u64,
    pub frame_count: u64,
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
    pub audio: Option<RecordingJournalAudioConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingJournalAudioConfig {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub encoder: String,
    pub pre_skip_frames: u16,
    pub codec_delay_ns: u64,
    pub seek_pre_roll_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RecordingTrackStats {
    pub video_frame_count: u64,
    pub audio_packet_count: Option<u64>,
    pub audio_pcm_frame_count: Option<u64>,
}

impl RecordingTrackStats {
    pub const fn video_only(video_frame_count: u64) -> Self {
        Self {
            video_frame_count,
            audio_packet_count: None,
            audio_pcm_frame_count: None,
        }
    }

    #[allow(dead_code)] // 下一提交由双轨 session writer 使用；本切片先固定持久化 API。
    pub const fn with_audio(
        video_frame_count: u64,
        audio_packet_count: u64,
        audio_pcm_frame_count: u64,
    ) -> Self {
        Self {
            video_frame_count,
            audio_packet_count: Some(audio_packet_count),
            audio_pcm_frame_count: Some(audio_pcm_frame_count),
        }
    }

    fn audio(self) -> Result<Option<AudioArtifactStats>, String> {
        match (self.audio_packet_count, self.audio_pcm_frame_count) {
            (None, None) => Ok(None),
            (Some(packet_count), Some(pcm_frame_count))
                if packet_count > 0 && pcm_frame_count > 0 =>
            {
                Ok(Some(AudioArtifactStats {
                    packet_count,
                    pcm_frame_count,
                }))
            }
            _ => Err("录屏音轨统计无效".to_string()),
        }
    }
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

        let audio = config.audio.map(|audio| AudioSpec {
            sample_rate_hz: audio.sample_rate_hz,
            channels: audio.channels,
            encoder: audio.encoder,
            pre_skip_frames: audio.pre_skip_frames,
            codec_delay_ns: audio.codec_delay_ns,
            seek_pre_roll_ns: audio.seek_pre_roll_ns,
        });
        let manifest = RecordingManifest {
            format: FORMAT.to_string(),
            schema_version: if audio.is_some() {
                AV_SCHEMA_VERSION
            } else {
                SCHEMA_VERSION
            },
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
            audio,
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
        pending: PendingSegment,
        file: File,
        duration_ns: u64,
        frame_count: u64,
        dropped_frames: u64,
    ) -> Result<PathBuf, String> {
        self.commit_segment_with_tracks(
            pending,
            file,
            duration_ns,
            RecordingTrackStats::video_only(frame_count),
            dropped_frames,
        )
    }

    pub fn commit_segment_with_tracks(
        &mut self,
        mut pending: PendingSegment,
        file: File,
        duration_ns: u64,
        tracks: RecordingTrackStats,
        dropped_frames: u64,
    ) -> Result<PathBuf, String> {
        if self.manifest.state != RecordingState::Recording
            || pending.index as usize != self.manifest.segments.len()
        {
            return Err("录屏分段与当前会话状态不一致".to_string());
        }
        let audio = tracks.audio()?;
        if duration_ns == 0
            || tracks.video_frame_count == 0
            || audio.is_some() != self.manifest.audio.is_some()
        {
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
            frame_count: tracks.video_frame_count,
            audio,
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
        if self.manifest.state != RecordingState::Recording || self.manifest.final_output.is_some()
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
        pending: PendingFinalOutput,
        file: File,
        duration_ns: u64,
        frame_count: u64,
    ) -> Result<PathBuf, String> {
        self.commit_final_output_with_tracks(
            pending,
            file,
            duration_ns,
            RecordingTrackStats::video_only(frame_count),
        )
    }

    pub fn commit_final_output_with_tracks(
        &mut self,
        mut pending: PendingFinalOutput,
        file: File,
        duration_ns: u64,
        tracks: RecordingTrackStats,
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
        let audio = tracks.audio()?;
        let expected_audio_frames =
            self.manifest
                .segments
                .iter()
                .try_fold(0_u64, |total, segment| {
                    let frames = segment
                        .audio
                        .as_ref()
                        .map_or(0, |audio| audio.pcm_frame_count);
                    total
                        .checked_add(frames)
                        .ok_or_else(|| "录屏最终音频帧数溢出".to_string())
                })?;
        if duration_ns != expected_duration
            || tracks.video_frame_count != expected_frames
            || audio.is_some() != self.manifest.audio.is_some()
            || audio
                .as_ref()
                .is_some_and(|audio| audio.pcm_frame_count != expected_audio_frames)
        {
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
            frame_count: tracks.video_frame_count,
            audio,
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

/// 返回可由结果库展示的稳定投影。损坏或越界的单个会话只记录警告，不阻断其余结果。
pub(super) fn list_library(app_data_dir: &Path) -> Result<Vec<RecordingLibraryItem>, String> {
    let root = app_data_dir.join(RECORDINGS_DIRECTORY);
    let Some(entries) = bounded_session_entries(&root)? else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for entry in entries {
        match library_item_for_session(&entry.path(), &entry.file_name()) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(error) => log::warn!("结果库跳过录屏会话 {}: {error}", entry.path().display()),
        }
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at_unix_ms));
    Ok(items)
}

pub(super) fn resolve_library_artifact(
    app_data_dir: &Path,
    session_id: &str,
    artifact_id: &str,
) -> Result<ResolvedRecordingArtifact, String> {
    let (session_directory, manifest) = load_library_manifest(app_data_dir, session_id)?;
    let (file_name, byte_length, sha256, suffix) = if artifact_id == "final" {
        if manifest.state != RecordingState::Complete {
            return Err("中断会话没有完整录屏产物".to_string());
        }
        let output = manifest
            .final_output
            .as_ref()
            .ok_or_else(|| "完成会话缺少最终输出".to_string())?;
        (
            output.file_name.as_str(),
            output.byte_length,
            output.sha256.as_str(),
            String::new(),
        )
    } else {
        if manifest.state != RecordingState::Interrupted {
            return Err("完成会话只允许访问最终输出".to_string());
        }
        let index = artifact_id
            .strip_prefix("segment-")
            .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| "录屏产物身份无效".to_string())?;
        let segment = manifest
            .segments
            .get(index as usize)
            .filter(|segment| segment.index == index)
            .ok_or_else(|| "录屏分段不存在".to_string())?;
        (
            segment.file_name.as_str(),
            segment.byte_length,
            segment.sha256.as_str(),
            format!("-segment-{index:06}"),
        )
    };
    let path = session_directory.join(file_name);
    ensure_artifact_metadata(&path, byte_length)?;
    Ok(ResolvedRecordingArtifact {
        path,
        suggested_file_name: format!("Clippy-{session_id}{suffix}.{}", manifest.video.container),
        byte_length,
        sha256: sha256.to_string(),
    })
}

#[cfg(feature = "recording-vp9-prototype")]
pub(super) fn resolve_library_thumbnail_source(
    app_data_dir: &Path,
    session_id: &str,
) -> Result<Option<RecordingThumbnailSource>, String> {
    let (session_directory, manifest) = load_library_manifest(app_data_dir, session_id)?;
    if manifest.video.encoder != "vp9-prototype"
        || manifest.video.container != "webm"
        || manifest.audio.is_some()
    {
        return Ok(None);
    }
    let (file_name, byte_length, sha256, duration_ns, frame_count) = match manifest.state {
        RecordingState::Complete => {
            let output = manifest
                .final_output
                .as_ref()
                .ok_or_else(|| "完成会话缺少最终输出".to_string())?;
            (
                output.file_name.as_str(),
                output.byte_length,
                output.sha256.as_str(),
                output.duration_ns,
                output.frame_count,
            )
        }
        RecordingState::Interrupted => {
            let Some(segment) = manifest.segments.first() else {
                return Ok(None);
            };
            (
                segment.file_name.as_str(),
                segment.byte_length,
                segment.sha256.as_str(),
                segment.duration_ns,
                segment.frame_count,
            )
        }
        RecordingState::Recording | RecordingState::Finalizing => return Ok(None),
    };
    let path = session_directory.join(file_name);
    ensure_artifact_metadata(&path, byte_length)?;
    Ok(Some(RecordingThumbnailSource {
        artifact: ResolvedRecordingArtifact {
            path,
            suggested_file_name: file_name.to_string(),
            byte_length,
            sha256: sha256.to_string(),
        },
        width: manifest.video.width,
        height: manifest.video.height,
        target_fps_numerator: manifest.video.target_fps_numerator,
        target_fps_denominator: manifest.video.target_fps_denominator,
        duration_ns,
        frame_count,
    }))
}

pub(super) fn verify_library_artifact(artifact: &ResolvedRecordingArtifact) -> Result<(), String> {
    verify_library_artifact_with_checkpoint(artifact, || Ok(()))
}

/// 把异常会话中已经提交的 VP9/WebM 分段无损 remux 为正常最终输出。
///
/// 分段始终保留；只有完整输出封尾、fsync 和哈希完成后才把清单提交为 `finalizing`。这样进程若在
/// 清单与文件提升之间退出，启动恢复可以沿用正常录屏的同一提交协议。
#[cfg(feature = "recording-vp9-prototype")]
pub(super) fn merge_interrupted_vp9_session(
    app_data_dir: &Path,
    session_id: &str,
) -> Result<(), String> {
    use super::mux::webm_remux::{remux_vp9_segments, WebmRemuxSource, WebmRemuxSpec};

    let (session_directory, mut manifest) = load_library_manifest(app_data_dir, session_id)?;
    if manifest.state != RecordingState::Interrupted
        || manifest.video.encoder != "vp9-prototype"
        || manifest.video.container != "webm"
        || manifest.audio.is_some()
        || manifest.segments.is_empty()
    {
        return Err("这次录屏不支持恢复合并".to_string());
    }

    let sources = manifest
        .segments
        .iter()
        .map(|segment| WebmRemuxSource {
            path: session_directory.join(&segment.file_name),
            byte_length: segment.byte_length,
            sha256: segment.sha256.clone(),
            started_at_ns: segment.started_at_ns,
            duration_ns: segment.duration_ns,
            frame_count: segment.frame_count,
        })
        .collect::<Vec<_>>();

    // 上次若在写清单以前失败，只可能留下这个固定名称的普通 partial。先清理再 create-new，不能让
    // 任意目录项或用户提供的路径参与合并。
    discard_final_output_files(&session_directory, &manifest.video.container)?;
    let partial_path = session_directory.join(final_output_partial_name("webm"));
    let output_file = create_private_new_file(&partial_path)
        .map_err(|error| format!("创建录屏恢复输出失败: {error}"))?;
    let mut manifest_committed = false;
    let result = (|| {
        let output = remux_vp9_segments(
            &sources,
            output_file,
            WebmRemuxSpec {
                width: manifest.video.width,
                height: manifest.video.height,
                fps_numerator: manifest.video.target_fps_numerator,
                fps_denominator: manifest.video.target_fps_denominator,
            },
        )?;
        let expected_duration = manifest
            .segments
            .last()
            .and_then(|segment| segment.started_at_ns.checked_add(segment.duration_ns))
            .ok_or_else(|| "录屏恢复总时长溢出".to_string())?;
        let expected_frames = manifest.segments.iter().try_fold(0_u64, |total, segment| {
            total
                .checked_add(segment.frame_count)
                .ok_or_else(|| "录屏恢复总帧数溢出".to_string())
        })?;
        if output.duration_ns != expected_duration || output.frame_count != expected_frames {
            return Err("录屏恢复输出与清单汇总不一致".to_string());
        }

        let file = output.writer;
        let metadata = file
            .metadata()
            .map_err(|error| format!("读取录屏恢复输出失败: {error}"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SESSION_BYTES {
            return Err("录屏恢复输出大小无效".to_string());
        }
        file.sync_all()
            .map_err(|error| format!("同步录屏恢复输出失败: {error}"))?;
        drop(file);
        restrict_file(&partial_path)
            .map_err(|error| format!("收紧录屏恢复输出权限失败: {error}"))?;
        let (byte_length, sha256) = hash_file_with_limit(&partial_path, MAX_SESSION_BYTES)?;

        let file_name = final_output_file_name("webm");
        manifest.final_output = Some(FinalOutputManifest {
            file_name: file_name.clone(),
            duration_ns: output.duration_ns,
            frame_count: output.frame_count,
            audio: None,
            byte_length,
            sha256,
        });
        manifest.recovery = None;
        manifest.state = RecordingState::Finalizing;
        validate_manifest(&manifest, session_id)?;
        write_manifest(&session_directory, &manifest)?;
        manifest_committed = true;

        let destination = session_directory.join(file_name);
        replace_private_file(&partial_path, &destination)
            .map_err(|error| format!("提交录屏恢复输出失败: {error}"))?;
        sync_directory(&session_directory)
            .map_err(|error| format!("同步录屏恢复目录失败: {error}"))?;
        manifest.state = RecordingState::Complete;
        write_manifest(&session_directory, &manifest)
    })();
    if result.is_err() && !manifest_committed {
        let _ = fs::remove_file(&partial_path);
    }
    result
}

pub(super) fn verify_library_artifact_with_checkpoint<F>(
    artifact: &ResolvedRecordingArtifact,
    checkpoint: F,
) -> Result<(), String>
where
    F: FnMut() -> Result<(), String>,
{
    ensure_artifact_metadata(&artifact.path, artifact.byte_length)?;
    let (byte_length, sha256) =
        hash_file_with_limit_and_checkpoint(&artifact.path, artifact.byte_length, checkpoint)?;
    if byte_length != artifact.byte_length || sha256 != artifact.sha256 {
        return Err("录屏产物与恢复清单不一致".to_string());
    }
    Ok(())
}

/// 导出时只读受清单约束的源文件，先在目标目录写完整临时文件，核对哈希后再替换目标。
pub(super) fn export_library_artifact(
    artifact: &ResolvedRecordingArtifact,
    destination: &Path,
) -> Result<(), String> {
    if destination == artifact.path {
        return Err("不能用导出文件覆盖内部恢复产物".to_string());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "导出位置缺少父目录".to_string())?;
    let parent_metadata =
        fs::metadata(parent).map_err(|error| format!("读取导出目录失败: {error}"))?;
    if !parent_metadata.is_dir() {
        return Err("导出位置不是目录".to_string());
    }
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("导出目标不是普通文件".to_string());
        }
    }
    ensure_artifact_metadata(&artifact.path, artifact.byte_length)?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".clippy-recording-export-{}-{nonce}.tmp",
        std::process::id()
    ));
    let result = (|| {
        let source =
            File::open(&artifact.path).map_err(|error| format!("打开录屏产物失败: {error}"))?;
        let mut source = BufReader::new(source);
        let mut output = create_private_new_file(&temporary)
            .map_err(|error| format!("创建导出临时文件失败: {error}"))?;
        let mut hasher = Sha256::new();
        let mut byte_length = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source
                .read(&mut buffer)
                .map_err(|error| format!("读取录屏产物失败: {error}"))?;
            if read == 0 {
                break;
            }
            byte_length = byte_length
                .checked_add(read as u64)
                .ok_or_else(|| "录屏产物大小溢出".to_string())?;
            if byte_length > artifact.byte_length {
                return Err("录屏产物大小与恢复清单不一致".to_string());
            }
            hasher.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("写入导出临时文件失败: {error}"))?;
        }
        let sha256 = format!("{:x}", hasher.finalize());
        if byte_length != artifact.byte_length || sha256 != artifact.sha256 {
            return Err("录屏产物与恢复清单不一致".to_string());
        }
        output
            .sync_all()
            .map_err(|error| format!("同步导出临时文件失败: {error}"))?;
        drop(output);
        replace_private_file(&temporary, destination)
            .map_err(|error| format!("提交录屏导出失败: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn delete_library_session(app_data_dir: &Path, session_id: &str) -> Result<(), String> {
    let (session_directory, manifest) = load_library_manifest(app_data_dir, session_id)?;
    let mut expected = HashSet::from([MANIFEST_FILE.to_string()]);
    for segment in &manifest.segments {
        let artifact = ResolvedRecordingArtifact {
            path: session_directory.join(&segment.file_name),
            suggested_file_name: segment.file_name.clone(),
            byte_length: segment.byte_length,
            sha256: segment.sha256.clone(),
        };
        verify_library_artifact(&artifact)?;
        expected.insert(segment.file_name.clone());
    }
    if let Some(output) = &manifest.final_output {
        let artifact = ResolvedRecordingArtifact {
            path: session_directory.join(&output.file_name),
            suggested_file_name: output.file_name.clone(),
            byte_length: output.byte_length,
            sha256: output.sha256.clone(),
        };
        verify_library_artifact(&artifact)?;
        expected.insert(output.file_name.clone());
    }

    let mut observed = HashSet::new();
    let mut entries = fs::read_dir(&session_directory)
        .map_err(|error| format!("读取录屏会话目录失败: {error}"))?;
    for _ in 0..=MAX_DIRECTORY_ENTRIES {
        let Some(entry) = entries
            .next()
            .transpose()
            .map_err(|error| format!("读取录屏会话目录项失败: {error}"))?
        else {
            break;
        };
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "录屏会话包含非 Unicode 文件名".to_string())?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("读取录屏会话文件失败: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || !expected.contains(&name) {
            return Err("录屏会话包含未知或不安全文件，拒绝删除".to_string());
        }
        observed.insert(name);
    }
    if entries.next().is_some() || observed != expected {
        return Err("录屏会话文件集合与恢复清单不一致".to_string());
    }
    for file_name in expected
        .iter()
        .filter(|name| name.as_str() != MANIFEST_FILE)
    {
        fs::remove_file(session_directory.join(file_name))
            .map_err(|error| format!("删除录屏产物失败: {error}"))?;
    }
    fs::remove_file(session_directory.join(MANIFEST_FILE))
        .map_err(|error| format!("删除录屏清单失败: {error}"))?;
    fs::remove_dir(&session_directory).map_err(|error| format!("删除录屏会话目录失败: {error}"))?;
    sync_directory(&app_data_dir.join(RECORDINGS_DIRECTORY))
        .map_err(|error| format!("同步录屏结果目录失败: {error}"))
}

/// 删除尚未成功启动的空会话。
///
/// 这个入口只供录屏启动事务回滚使用：原生帧源（例如 Wayland Portal）尚未建立时，编码器可能
/// 已经创建清单和临时文件。调用方必须先回收编码 worker；这里随后只接受没有任何已提交帧、
/// 最终输出或恢复信息，且目录中仅剩清单的会话，避免把启动回滚扩大成任意结果删除。
pub(super) fn discard_unstarted_session(
    app_data_dir: &Path,
    session_id: &str,
) -> Result<(), String> {
    if !valid_identifier(session_id, 64) {
        return Err("录屏会话身份无效".to_string());
    }
    let root = app_data_dir.join(RECORDINGS_DIRECTORY);
    let root_metadata =
        fs::symlink_metadata(&root).map_err(|error| format!("读取录屏结果目录失败: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("录屏结果路径不是普通目录".to_string());
    }
    let session_directory = root.join(session_id);
    let metadata = fs::symlink_metadata(&session_directory)
        .map_err(|error| format!("读取录屏会话失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("录屏会话路径不是普通目录".to_string());
    }
    let manifest_path = session_directory.join(MANIFEST_FILE);
    let manifest = read_manifest(&manifest_path)?;
    validate_manifest(&manifest, session_id)?;
    if !matches!(
        manifest.state,
        RecordingState::Recording | RecordingState::Interrupted
    ) || !manifest.segments.is_empty()
        || manifest.final_output.is_some()
        || manifest.recovery.is_some()
        || manifest.dropped_frames != 0
    {
        return Err("录屏会话已经包含可恢复内容，拒绝按启动失败删除".to_string());
    }

    let mut entries = fs::read_dir(&session_directory)
        .map_err(|error| format!("读取录屏会话目录失败: {error}"))?;
    let Some(entry) = entries
        .next()
        .transpose()
        .map_err(|error| format!("读取录屏会话目录项失败: {error}"))?
    else {
        return Err("录屏启动会话缺少清单".to_string());
    };
    let entry_metadata = fs::symlink_metadata(entry.path())
        .map_err(|error| format!("读取录屏启动文件失败: {error}"))?;
    if entry.file_name() != std::ffi::OsStr::new(MANIFEST_FILE)
        || entry_metadata.file_type().is_symlink()
        || !entry_metadata.is_file()
        || entries.next().is_some()
    {
        return Err("录屏启动会话包含未知或不安全文件，拒绝删除".to_string());
    }

    fs::remove_file(&manifest_path).map_err(|error| format!("删除录屏启动清单失败: {error}"))?;
    fs::remove_dir(&session_directory).map_err(|error| format!("删除录屏启动会话失败: {error}"))?;
    sync_directory(&root).map_err(|error| format!("同步录屏结果目录失败: {error}"))
}

fn bounded_session_entries(root: &Path) -> Result<Option<Vec<fs::DirEntry>>, String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取录屏结果目录失败: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("录屏结果路径不是普通目录".to_string());
    }
    restrict_directory(root).map_err(|error| format!("收紧录屏结果目录权限失败: {error}"))?;
    let mut entries = Vec::new();
    let mut reader = fs::read_dir(root).map_err(|error| format!("读取录屏结果失败: {error}"))?;
    for _ in 0..=MAX_DIRECTORY_ENTRIES {
        let Some(entry) = reader
            .next()
            .transpose()
            .map_err(|error| format!("读取录屏结果项失败: {error}"))?
        else {
            break;
        };
        entries.push(entry);
    }
    if entries.len() > MAX_DIRECTORY_ENTRIES {
        entries.truncate(MAX_DIRECTORY_ENTRIES);
        log::warn!(
            "录屏结果目录超过 {} 个条目，只扫描有界前缀",
            MAX_DIRECTORY_ENTRIES
        );
    }
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            fs::symlink_metadata(entry.path())
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    if entries.len() > MAX_SESSIONS {
        entries.truncate(MAX_SESSIONS);
        log::warn!("录屏结果数量超过 {} 个，只显示最新会话", MAX_SESSIONS);
    }
    Ok(Some(entries))
}

fn library_item_for_session(
    path: &Path,
    directory_name: &std::ffi::OsStr,
) -> Result<Option<RecordingLibraryItem>, String> {
    let session_id = directory_name
        .to_str()
        .filter(|value| valid_identifier(value, 64))
        .ok_or_else(|| "会话目录名无效".to_string())?;
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("读取会话失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("会话路径不是普通目录".to_string());
    }
    let manifest = read_manifest(&path.join(MANIFEST_FILE))?;
    validate_manifest(&manifest, session_id)?;
    if !matches!(
        manifest.state,
        RecordingState::Complete | RecordingState::Interrupted
    ) {
        return Ok(None);
    }

    let mut artifacts = Vec::new();
    match manifest.state {
        RecordingState::Complete => {
            let output = manifest
                .final_output
                .as_ref()
                .ok_or_else(|| "完成会话缺少最终输出".to_string())?;
            ensure_artifact_metadata(&path.join(&output.file_name), output.byte_length)?;
            artifacts.push(RecordingLibraryArtifact {
                artifact_id: "final".to_string(),
                display_name: output.file_name.clone(),
                duration_ms: output.duration_ns / 1_000_000,
                frame_count: output.frame_count,
                byte_length: output.byte_length,
            });
        }
        RecordingState::Interrupted => {
            for segment in &manifest.segments {
                ensure_artifact_metadata(&path.join(&segment.file_name), segment.byte_length)?;
                artifacts.push(RecordingLibraryArtifact {
                    artifact_id: format!("segment-{:06}", segment.index),
                    display_name: segment.file_name.clone(),
                    duration_ms: segment.duration_ns / 1_000_000,
                    frame_count: segment.frame_count,
                    byte_length: segment.byte_length,
                });
            }
        }
        RecordingState::Recording | RecordingState::Finalizing => return Ok(None),
    }
    let duration_ms = match manifest.state {
        RecordingState::Complete => manifest
            .final_output
            .as_ref()
            .map(|output| output.duration_ns / 1_000_000)
            .unwrap_or(0),
        RecordingState::Interrupted => manifest
            .segments
            .last()
            .and_then(|segment| segment.started_at_ns.checked_add(segment.duration_ns))
            .map(|duration| duration / 1_000_000)
            .unwrap_or(0),
        RecordingState::Recording | RecordingState::Finalizing => unreachable!(),
    };
    let frame_count = artifacts.iter().map(|artifact| artifact.frame_count).sum();
    let byte_length = artifacts.iter().map(|artifact| artifact.byte_length).sum();
    let can_merge = cfg!(feature = "recording-vp9-prototype")
        && manifest.state == RecordingState::Interrupted
        && manifest.video.encoder == "vp9-prototype"
        && manifest.video.container == "webm"
        && manifest.audio.is_none()
        && !manifest.segments.is_empty();
    let can_thumbnail = cfg!(feature = "recording-vp9-prototype")
        && manifest.video.encoder == "vp9-prototype"
        && manifest.video.container == "webm"
        && manifest.audio.is_none()
        && !artifacts.is_empty();
    let audio = manifest.audio.map(|audio| RecordingLibraryAudio {
        sample_rate_hz: audio.sample_rate_hz,
        channels: audio.channels,
        encoder: audio.encoder,
    });
    Ok(Some(RecordingLibraryItem {
        session_id: manifest.session_id,
        state: match manifest.state {
            RecordingState::Complete => "complete",
            RecordingState::Interrupted => "interrupted",
            RecordingState::Recording | RecordingState::Finalizing => unreachable!(),
        },
        created_at_unix_ms: manifest.created_at_unix_ms,
        width: manifest.video.width,
        height: manifest.video.height,
        target_fps_numerator: manifest.video.target_fps_numerator,
        target_fps_denominator: manifest.video.target_fps_denominator,
        encoder: manifest.video.encoder,
        container: manifest.video.container,
        include_cursor: manifest.video.include_cursor,
        audio,
        dropped_frames: manifest.dropped_frames,
        duration_ms,
        frame_count,
        byte_length,
        can_merge,
        can_thumbnail,
        artifacts,
    }))
}

fn load_library_manifest(
    app_data_dir: &Path,
    session_id: &str,
) -> Result<(PathBuf, RecordingManifest), String> {
    if !valid_identifier(session_id, 64) {
        return Err("录屏会话身份无效".to_string());
    }
    let root = app_data_dir.join(RECORDINGS_DIRECTORY);
    let root_metadata =
        fs::symlink_metadata(&root).map_err(|error| format!("读取录屏结果目录失败: {error}"))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("录屏结果路径不是普通目录".to_string());
    }
    let session_directory = root.join(session_id);
    let metadata = fs::symlink_metadata(&session_directory)
        .map_err(|error| format!("读取录屏会话失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("录屏会话路径不是普通目录".to_string());
    }
    let manifest = read_manifest(&session_directory.join(MANIFEST_FILE))?;
    validate_manifest(&manifest, session_id)?;
    if !matches!(
        manifest.state,
        RecordingState::Complete | RecordingState::Interrupted
    ) {
        return Err("活动录屏不能由结果库访问".to_string());
    }
    Ok((session_directory, manifest))
}

fn ensure_artifact_metadata(path: &Path, byte_length: u64) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("读取录屏产物失败: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("录屏产物不是普通文件".to_string());
    }
    if metadata.len() != byte_length {
        return Err("录屏产物大小与恢复清单不一致".to_string());
    }
    restrict_file(path).map_err(|error| format!("收紧录屏产物权限失败: {error}"))
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
    if manifest.state == RecordingState::Interrupted {
        // 恢复 remux 若在提交清单以前被进程终止，只会留下固定名称的 final partial。Interrupted
        // 清单仍是事实源；启动时清掉该临时文件，避免结果库删除永远被未知目录项阻塞。
        discard_final_output_files(path, &manifest.video.container)?;
        return Ok(None);
    }
    if manifest.state == RecordingState::Complete {
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
    if manifest.format != FORMAT
        || !matches!(manifest.schema_version, SCHEMA_VERSION | AV_SCHEMA_VERSION)
    {
        return Err("清单格式或 schema 版本不受支持".to_string());
    }
    match (manifest.schema_version, manifest.audio.as_ref()) {
        (SCHEMA_VERSION, None) => {}
        (AV_SCHEMA_VERSION, Some(audio)) if valid_audio_spec(audio, &manifest.video) => {}
        _ => return Err("清单 schema 与音轨描述不一致".to_string()),
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
    let mut total_audio_frames = 0_u64;
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
        if !valid_audio_artifact_stats(segment.audio.as_ref(), manifest.audio.is_some()) {
            return Err("清单分段音轨统计无效".to_string());
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
        total_audio_frames = total_audio_frames
            .checked_add(
                segment
                    .audio
                    .as_ref()
                    .map_or(0, |audio| audio.pcm_frame_count),
            )
            .ok_or_else(|| "清单分段音频帧数溢出".to_string())?;
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
            || !valid_audio_artifact_stats(output.audio.as_ref(), manifest.audio.is_some())
            || output
                .audio
                .as_ref()
                .is_some_and(|audio| audio.pcm_frame_count != total_audio_frames)
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

fn valid_audio_spec(audio: &AudioSpec, video: &VideoSpec) -> bool {
    audio.sample_rate_hz == OPUS_SAMPLE_RATE_HZ
        && matches!(audio.channels, 1 | 2)
        && audio.encoder == "opus"
        && audio.pre_skip_frames > 0
        && audio.codec_delay_ns
            == u64::from(audio.pre_skip_frames) * TIMEBASE_HZ / u64::from(OPUS_SAMPLE_RATE_HZ)
        && audio.seek_pre_roll_ns == OPUS_SEEK_PRE_ROLL_NS
        && video.encoder == "vp9-prototype"
        && video.container == "webm"
}

fn valid_audio_artifact_stats(stats: Option<&AudioArtifactStats>, audio_expected: bool) -> bool {
    match (stats, audio_expected) {
        (None, false) => true,
        (Some(stats), true) => stats.packet_count > 0 && stats.pcm_frame_count > 0,
        _ => false,
    }
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
    hash_file_with_limit_and_checkpoint(path, byte_limit, || Ok(()))
}

fn hash_file_with_limit_and_checkpoint<F>(
    path: &Path,
    byte_limit: u64,
    mut checkpoint: F,
) -> Result<(u64, String), String>
where
    F: FnMut() -> Result<(), String>,
{
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
        checkpoint()?;
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
    #[cfg(feature = "recording-vp9-prototype")]
    use std::io::Seek;

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
            audio: None,
            dropped_frames: 0,
            segments: Vec::new(),
            final_output: None,
            recovery: None,
        }
    }

    fn av_audio_spec() -> AudioSpec {
        AudioSpec {
            sample_rate_hz: OPUS_SAMPLE_RATE_HZ,
            channels: 2,
            encoder: "opus".to_string(),
            pre_skip_frames: 312,
            codec_delay_ns: 6_500_000,
            seek_pre_roll_ns: OPUS_SEEK_PRE_ROLL_NS,
        }
    }

    fn av_journal_config(session_id: &str) -> RecordingJournalConfig {
        RecordingJournalConfig {
            session_id: session_id.to_string(),
            source_id: "display-av".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 640,
            height: 480,
            target_fps_numerator: 30,
            target_fps_denominator: 1,
            encoder: "vp9-prototype".to_string(),
            container: "webm".to_string(),
            include_cursor: true,
            audio: Some(RecordingJournalAudioConfig {
                sample_rate_hz: OPUS_SAMPLE_RATE_HZ,
                channels: 2,
                encoder: "opus".to_string(),
                pre_skip_frames: 312,
                codec_delay_ns: 6_500_000,
                seek_pre_roll_ns: OPUS_SEEK_PRE_ROLL_NS,
            }),
        }
    }

    fn write_manifest_fixture(directory: &Path, manifest: &RecordingManifest) {
        fs::write(
            directory.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(manifest).unwrap(),
        )
        .unwrap();
    }

    #[cfg(feature = "recording-vp9-prototype")]
    fn vp9_journal_config(session_id: &str) -> RecordingJournalConfig {
        RecordingJournalConfig {
            session_id: session_id.to_string(),
            source_id: "display-vp9".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 64,
            height: 48,
            target_fps_numerator: 10,
            target_fps_denominator: 1,
            encoder: "vp9-prototype".to_string(),
            container: "webm".to_string(),
            include_cursor: true,
            audio: None,
        }
    }

    #[cfg(feature = "recording-vp9-prototype")]
    fn commit_vp9_segment(journal: &mut RecordingJournal, color: [u8; 3]) {
        use crate::recording::mux::vp9_webm::Vp9WebmWriter;

        let rgba = (0..64 * 48)
            .flat_map(|_| [color[0], color[1], color[2], 255])
            .collect::<Vec<_>>();
        let (file, pending) = journal.begin_segment().unwrap();
        let mut writer = Vp9WebmWriter::new(file, 64, 48, 10, 1).unwrap();
        writer.push_rgba(&rgba, 0).unwrap();
        writer.push_rgba(&rgba, 100_000_000).unwrap();
        let mut output = writer.finish_with_stats(200_000_000).unwrap();
        output.writer.rewind().unwrap();
        journal
            .commit_segment(pending, output.writer, 200_000_000, output.frame_count, 0)
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
            audio: None,
            byte_length: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        });
    }

    fn add_av_segment(directory: &Path, manifest: &mut RecordingManifest, bytes: &[u8]) {
        let index = manifest.segments.len() as u32;
        let file_name = format!("segment-{index:06}.{}", manifest.video.container);
        fs::write(directory.join(&file_name), bytes).unwrap();
        manifest.segments.push(SegmentManifest {
            index,
            file_name,
            started_at_ns: u64::from(index) * TIMEBASE_HZ,
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            audio: Some(AudioArtifactStats {
                packet_count: 50,
                pcm_frame_count: 48_000,
            }),
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

    fn create_complete_session(root: &Path, session_id: &str) -> std::path::PathBuf {
        let session = create_session(root, session_id);
        let mut manifest = fixture_manifest(session_id, RecordingState::Complete);
        add_segment(&session, &mut manifest, b"committed segment");
        let final_bytes = b"complete recording";
        fs::write(session.join("recording.webm"), final_bytes).unwrap();
        manifest.final_output = Some(FinalOutputManifest {
            file_name: "recording.webm".to_string(),
            duration_ns: TIMEBASE_HZ,
            frame_count: 30,
            audio: None,
            byte_length: final_bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(final_bytes)),
        });
        write_manifest_fixture(&session, &manifest);
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
            audio: None,
        }
    }

    #[test]
    fn video_only_journal_keeps_schema_v1_and_omits_audio_shape() {
        let temporary = tempfile::tempdir().unwrap();
        let journal = RecordingJournal::create(temporary.path(), journal_config("video-v1"))
            .expect("纯视频 journal 应保持兼容");
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(journal.session_directory().join(MANIFEST_FILE)).unwrap(),
        )
        .unwrap();

        assert_eq!(value["schemaVersion"], SCHEMA_VERSION);
        assert!(value.get("audio").is_none());
        assert!(value["segments"].as_array().unwrap().is_empty());
    }

    #[test]
    fn av_journal_commits_schema_v2_with_atomic_track_statistics() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal = RecordingJournal::create(temporary.path(), av_journal_config("av-v2"))
            .expect("双轨 journal 应创建");
        let session = journal.session_directory().to_path_buf();

        let (mut segment_file, pending_segment) = journal.begin_segment().unwrap();
        segment_file.write_all(b"dual-track-segment").unwrap();
        journal
            .commit_segment_with_tracks(
                pending_segment,
                segment_file,
                TIMEBASE_HZ,
                RecordingTrackStats::with_audio(30, 50, 48_000),
                3,
            )
            .unwrap();

        let (mut final_file, pending_final) = journal.begin_final_output().unwrap();
        final_file.write_all(b"dual-track-final").unwrap();
        journal
            .commit_final_output_with_tracks(
                pending_final,
                final_file,
                TIMEBASE_HZ,
                RecordingTrackStats::with_audio(30, 51, 48_000),
            )
            .unwrap();
        journal.complete().unwrap();

        let manifest = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.schema_version, AV_SCHEMA_VERSION);
        assert_eq!(manifest.audio, Some(av_audio_spec()));
        assert_eq!(
            manifest.segments[0].audio,
            Some(AudioArtifactStats {
                packet_count: 50,
                pcm_frame_count: 48_000,
            })
        );
        assert_eq!(
            manifest.final_output.unwrap().audio,
            Some(AudioArtifactStats {
                packet_count: 51,
                pcm_frame_count: 48_000,
            })
        );

        let item = list_library(temporary.path()).unwrap().remove(0);
        assert_eq!(
            item.audio,
            Some(RecordingLibraryAudio {
                sample_rate_hz: OPUS_SAMPLE_RATE_HZ,
                channels: 2,
                encoder: "opus".to_string(),
            })
        );
        assert!(!item.can_merge);
        assert!(!item.can_thumbnail);
        #[cfg(feature = "recording-vp9-prototype")]
        assert!(resolve_library_thumbnail_source(temporary.path(), "av-v2")
            .unwrap()
            .is_none());
    }

    #[test]
    fn manifest_rejects_mixed_v1_v2_shapes_and_incomplete_audio_statistics() {
        let mut legacy = fixture_manifest("legacy-mixed", RecordingState::Recording);
        legacy.audio = Some(av_audio_spec());
        assert!(validate_manifest(&legacy, "legacy-mixed").is_err());

        let mut missing_track = fixture_manifest("v2-missing", RecordingState::Recording);
        missing_track.schema_version = AV_SCHEMA_VERSION;
        assert!(validate_manifest(&missing_track, "v2-missing").is_err());

        let mut invalid_audio = fixture_manifest("v2-invalid", RecordingState::Recording);
        invalid_audio.schema_version = AV_SCHEMA_VERSION;
        invalid_audio.audio = Some(av_audio_spec());
        invalid_audio.audio.as_mut().unwrap().sample_rate_hz = 44_100;
        assert!(validate_manifest(&invalid_audio, "v2-invalid").is_err());
        invalid_audio.audio = Some(av_audio_spec());
        invalid_audio.audio.as_mut().unwrap().codec_delay_ns = 6_000_000;
        assert!(validate_manifest(&invalid_audio, "v2-invalid").is_err());

        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "v2-stats");
        let mut missing_stats = fixture_manifest("v2-stats", RecordingState::Interrupted);
        missing_stats.schema_version = AV_SCHEMA_VERSION;
        missing_stats.audio = Some(av_audio_spec());
        add_segment(&session, &mut missing_stats, b"video-shaped segment");
        assert!(validate_manifest(&missing_stats, "v2-stats").is_err());

        missing_stats.segments[0].audio = Some(AudioArtifactStats {
            packet_count: 0,
            pcm_frame_count: 48_000,
        });
        assert!(validate_manifest(&missing_stats, "v2-stats").is_err());
    }

    #[test]
    fn recovery_keeps_verified_av_prefix_and_truncates_tampered_tail() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "av-recovery");
        let mut manifest = fixture_manifest("av-recovery", RecordingState::Recording);
        manifest.schema_version = AV_SCHEMA_VERSION;
        manifest.video.encoder = "vp9-prototype".to_string();
        manifest.audio = Some(av_audio_spec());
        add_av_segment(&session, &mut manifest, b"valid dual-track prefix");
        add_av_segment(&session, &mut manifest, b"expected dual-track tail");
        fs::write(
            session.join("segment-000001.webm"),
            b"tampered dual-track tail",
        )
        .unwrap();
        write_manifest_fixture(&session, &manifest);

        let summary = recover_interrupted_sessions(temporary.path()).unwrap();
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 1);
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert_eq!(recovered.schema_version, AV_SCHEMA_VERSION);
        assert_eq!(recovered.segments.len(), 1);
        assert_eq!(
            recovered.segments[0].audio.as_ref().unwrap().packet_count,
            50
        );
        assert_eq!(recovered.recovery.unwrap().first_invalid_segment, Some(1));

        let item = list_library(temporary.path()).unwrap().remove(0);
        assert!(item.audio.is_some());
        assert!(!item.can_merge);
        assert!(!item.can_thumbnail);
        #[cfg(feature = "recording-vp9-prototype")]
        assert!(merge_interrupted_vp9_session(temporary.path(), "av-recovery").is_err());
    }

    #[test]
    fn library_lists_only_complete_and_interrupted_sessions_without_paths() {
        let temporary = tempfile::tempdir().unwrap();
        create_complete_session(temporary.path(), "complete-1");

        let interrupted = create_session(temporary.path(), "interrupted-1");
        let mut interrupted_manifest =
            fixture_manifest("interrupted-1", RecordingState::Interrupted);
        add_segment(
            &interrupted,
            &mut interrupted_manifest,
            b"recoverable segment",
        );
        write_manifest_fixture(&interrupted, &interrupted_manifest);

        let active = create_session(temporary.path(), "active-1");
        write_manifest_fixture(
            &active,
            &fixture_manifest("active-1", RecordingState::Recording),
        );

        let items = list_library(temporary.path()).unwrap();
        assert_eq!(items.len(), 2);
        let complete = items
            .iter()
            .find(|item| item.session_id == "complete-1")
            .unwrap();
        assert_eq!(complete.state, "complete");
        assert!(!complete.can_merge);
        assert!(!complete.can_thumbnail);
        assert_eq!(complete.artifacts[0].artifact_id, "final");
        assert_eq!(complete.artifacts[0].display_name, "recording.webm");
        let interrupted = items
            .iter()
            .find(|item| item.session_id == "interrupted-1")
            .unwrap();
        assert_eq!(interrupted.state, "interrupted");
        assert!(!interrupted.can_merge);
        assert!(!interrupted.can_thumbnail);
        assert_eq!(interrupted.artifacts[0].artifact_id, "segment-000000");
    }

    #[test]
    fn library_export_verifies_hash_before_replacing_destination() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_complete_session(temporary.path(), "export-1");
        let artifact = resolve_library_artifact(temporary.path(), "export-1", "final").unwrap();
        let destination = temporary.path().join("exported.webm");
        export_library_artifact(&artifact, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"complete recording");

        fs::write(session.join("recording.webm"), b"tampered recording").unwrap();
        let protected = temporary.path().join("protected.webm");
        fs::write(&protected, b"keep me").unwrap();
        assert!(export_library_artifact(&artifact, &protected).is_err());
        assert_eq!(fs::read(protected).unwrap(), b"keep me");
    }

    #[test]
    fn library_delete_rejects_unknown_files_then_removes_valid_session() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_complete_session(temporary.path(), "delete-1");
        fs::write(session.join("unexpected.txt"), b"do not delete").unwrap();
        assert!(delete_library_session(temporary.path(), "delete-1").is_err());
        assert!(session.exists());

        fs::remove_file(session.join("unexpected.txt")).unwrap();
        delete_library_session(temporary.path(), "delete-1").unwrap();
        assert!(!session.exists());
    }

    #[test]
    fn library_rejects_active_session_access() {
        let temporary = tempfile::tempdir().unwrap();
        let active = create_session(temporary.path(), "active-delete");
        write_manifest_fixture(
            &active,
            &fixture_manifest("active-delete", RecordingState::Recording),
        );
        assert!(resolve_library_artifact(temporary.path(), "active-delete", "final").is_err());
        assert!(delete_library_session(temporary.path(), "active-delete").is_err());
    }

    #[test]
    fn startup_rollback_deletes_only_empty_session() {
        let temporary = tempfile::tempdir().unwrap();
        let journal =
            RecordingJournal::create(temporary.path(), journal_config("startup-empty")).unwrap();
        let session = journal.session_directory().to_path_buf();
        drop(journal);

        discard_unstarted_session(temporary.path(), "startup-empty").unwrap();
        assert!(!session.exists());

        let protected =
            RecordingJournal::create(temporary.path(), journal_config("startup-protected"))
                .unwrap();
        let protected_session = protected.session_directory().to_path_buf();
        drop(protected);
        fs::write(protected_session.join("unexpected.txt"), b"keep").unwrap();

        assert!(discard_unstarted_session(temporary.path(), "startup-protected").is_err());
        assert!(protected_session.exists());
        assert_eq!(
            fs::read(protected_session.join("unexpected.txt")).unwrap(),
            b"keep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn library_rejects_symlinked_artifacts() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let session = create_complete_session(temporary.path(), "symlink-1");
        let outside = temporary.path().join("outside.webm");
        fs::write(&outside, b"complete recording").unwrap();
        fs::remove_file(session.join("recording.webm")).unwrap();
        symlink(&outside, session.join("recording.webm")).unwrap();

        assert!(list_library(temporary.path()).unwrap().is_empty());
        assert!(delete_library_session(temporary.path(), "symlink-1").is_err());
        assert!(outside.exists());
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
        let serialized = String::from_utf8(fs::read(session.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert!(!serialized.contains("\"audio\""));
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
            audio: None,
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
            audio: None,
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
            audio: None,
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
    fn startup_removes_uncommitted_recovery_merge_output() {
        let temporary = tempfile::tempdir().unwrap();
        let session = create_session(temporary.path(), "merge-abandoned");
        let mut manifest = fixture_manifest("merge-abandoned", RecordingState::Interrupted);
        add_segment(&session, &mut manifest, b"verified segment");
        write_manifest_fixture(&session, &manifest);
        fs::write(session.join(".recording.webm.partial"), b"unfinished remux").unwrap();

        recover_interrupted_sessions(temporary.path()).unwrap();

        assert!(!session.join(".recording.webm.partial").exists());
        let recovered = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(recovered.state, RecordingState::Interrupted);
        assert!(recovered.final_output.is_none());
        assert!(session.join("segment-000000.webm").exists());
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

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn interrupted_vp9_segments_merge_into_a_complete_recording() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), vp9_journal_config("merge-two")).unwrap();
        commit_vp9_segment(&mut journal, [16, 32, 64]);
        commit_vp9_segment(&mut journal, [200, 180, 160]);
        journal.interrupt().unwrap();
        let interrupted = list_library(temporary.path()).unwrap().pop().unwrap();
        assert!(interrupted.can_merge);
        assert!(interrupted.can_thumbnail);
        let thumbnail = resolve_library_thumbnail_source(temporary.path(), "merge-two")
            .unwrap()
            .unwrap();
        assert!(thumbnail.artifact.path.ends_with("segment-000000.webm"));

        merge_interrupted_vp9_session(temporary.path(), "merge-two").unwrap();

        let session = temporary
            .path()
            .join(RECORDINGS_DIRECTORY)
            .join("merge-two");
        let manifest = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.state, RecordingState::Complete);
        assert!(manifest.recovery.is_none());
        assert_eq!(manifest.segments.len(), 2);
        let final_output = manifest.final_output.unwrap();
        assert_eq!(final_output.duration_ns, 400_000_000);
        assert_eq!(final_output.frame_count, 4);
        assert!(session.join("recording.webm").is_file());
        assert!(!session.join(".recording.webm.partial").exists());
        let item = list_library(temporary.path()).unwrap().pop().unwrap();
        assert_eq!(item.state, "complete");
        assert!(item.can_thumbnail);
        assert_eq!(item.artifacts[0].artifact_id, "final");
        let thumbnail = resolve_library_thumbnail_source(temporary.path(), "merge-two")
            .unwrap()
            .unwrap();
        assert!(thumbnail.artifact.path.ends_with("recording.webm"));
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn failed_recovery_merge_preserves_interrupted_manifest_and_segments() {
        let temporary = tempfile::tempdir().unwrap();
        let mut journal =
            RecordingJournal::create(temporary.path(), vp9_journal_config("merge-tampered"))
                .unwrap();
        commit_vp9_segment(&mut journal, [16, 32, 64]);
        journal.interrupt().unwrap();
        let session = temporary
            .path()
            .join(RECORDINGS_DIRECTORY)
            .join("merge-tampered");
        fs::write(session.join("segment-000000.webm"), b"tampered").unwrap();

        assert!(merge_interrupted_vp9_session(temporary.path(), "merge-tampered").is_err());

        let manifest = read_manifest(&session.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.state, RecordingState::Interrupted);
        assert!(manifest.final_output.is_none());
        assert!(session.join("segment-000000.webm").exists());
        assert!(!session.join("recording.webm").exists());
        assert!(!session.join(".recording.webm.partial").exists());
    }
}
