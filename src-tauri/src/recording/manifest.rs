use super::RecoverySummary;
use crate::private_files::{
    replace_private_file, restrict_directory, restrict_file, write_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::Path;
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
    recovery: Option<RecoveryInfo>,
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
    let verified_prefix = verify_segment_prefix(path, &manifest.segments)?;
    let first_invalid_segment = (verified_prefix < original_segment_count)
        .then(|| u32::try_from(verified_prefix).expect("分段上限保证可以转为 u32"));
    manifest.segments.truncate(verified_prefix);
    manifest.state = RecordingState::Interrupted;
    manifest.recovery = Some(RecoveryInfo {
        recovered_at_unix_ms: unix_time_ms(),
        original_segment_count: u32::try_from(original_segment_count)
            .expect("分段上限保证可以转为 u32"),
        first_invalid_segment,
    });
    write_manifest(path, &manifest)?;
    Ok(Some(verified_prefix))
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
        if total_bytes > MAX_SESSION_BYTES {
            return Err("清单会话大小超过恢复上限".to_string());
        }
    }
    Ok(())
}

fn verify_segment_prefix(path: &Path, segments: &[SegmentManifest]) -> Result<usize, String> {
    for (position, segment) in segments.iter().enumerate() {
        let segment_path = path.join(&segment.file_name);
        let metadata = match fs::symlink_metadata(&segment_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(position),
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
    Ok(segments.len())
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
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
        if byte_length > MAX_SEGMENT_BYTES {
            return Err("分段超过恢复大小上限".to_string());
        }
        hasher.update(&buffer[..read]);
    }
    Ok((byte_length, format!("{:x}", hasher.finalize())))
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
