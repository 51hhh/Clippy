use super::*;
use crate::recording::mux::webm_remux::{
    visit_vp9_opus_packets, ParsedAvPacket, WebmAvRemuxSource, WebmRemuxSource, WebmRemuxSpec,
    WebmThumbnailAudioSpec,
};
use sha2::{Digest, Sha256};
use std::fs;

const SEGMENT: &[u8] = &[0x18, 0x53, 0x80, 0x67];
const TRACKS: &[u8] = &[0x16, 0x54, 0xAE, 0x6B];
const TRACK_ENTRY: &[u8] = &[0xAE];
const CLUSTER: &[u8] = &[0x1F, 0x43, 0xB6, 0x75];
const DELAY: &[u8] = &[0x56, 0xAA];
const PRE_ROLL: &[u8] = &[0x56, 0xBB];
const PADDING: &[u8] = &[0x75, 0xA2];

// 结构负例只表达受测 EBML 子树，不作为可播放 WebM；真实媒体另由原 mux 产生。
fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_vec();
    bytes.push(0x01);
    bytes.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
    bytes.extend_from_slice(body);
    bytes
}

fn track_document(children: &[u8]) -> Vec<u8> {
    element(SEGMENT, &element(TRACKS, &element(TRACK_ENTRY, children)))
}

fn padding_document(children: &[u8]) -> Vec<u8> {
    element(SEGMENT, &element(CLUSTER, &element(&[0xA0], children)))
}

fn retained(name: &str, bytes: &[u8]) {
    if let Some(directory) = std::env::var_os("CLIPPY_EBML_EVIDENCE") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(name), bytes).unwrap();
    }
}

fn real_media() -> (Vec<u8>, OpusTrackConfig, OpusEncoderStats, u64) {
    let mut video = Vp9PacketEncoder::new(64, 48, 10, 1).unwrap();
    let mut video_packets = Vec::new();
    let mut collect = |data: &[u8], timestamp, keyframe| {
        video_packets.push((data.to_vec(), timestamp, keyframe));
        Ok(())
    };
    video
        .push_rgba(&vec![0x80; 64 * 48 * 4], 0, &mut collect)
        .unwrap();
    video.finish(100_000_000, &mut collect).unwrap();
    let mut audio = OpusPacketEncoder::new(2).unwrap();
    let config = audio.track_config().clone();
    let mut audio_packets = audio.push(queued(0, 0, 1_000, 2, 0)).unwrap();
    let finish = audio.finish().unwrap();
    audio_packets.extend(finish.packets);
    let padding = audio_packets.last().unwrap().discard_padding_ns;
    let mut mux = AvWebmPacketMux::new(Cursor::new(Vec::new()), 64, 48, &config).unwrap();
    for (data, timestamp, keyframe) in video_packets {
        mux.add_video_packet(&data, timestamp, keyframe).unwrap();
    }
    for packet in &audio_packets {
        mux.add_audio_packet(packet).unwrap();
    }
    let output = mux.finish(100_000_000).unwrap();
    (output.writer.into_inner(), config, finish.stats, padding)
}

// 原生产读取器完整遍历并检查长度/SHA/轨道字段/尾 padding；比较每包内容与时间。
fn strict_packets(bytes: &[u8], config: &OpusTrackConfig, stats: OpusEncoderStats) -> Vec<String> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.webm");
    fs::write(&path, bytes).unwrap();
    let source = WebmAvRemuxSource {
        source: WebmRemuxSource {
            path,
            byte_length: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            started_at_ns: 0,
            duration_ns: 100_000_000,
            frame_count: 1,
        },
        audio: WebmThumbnailAudioSpec {
            sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
            channels: config.channels,
            pre_skip_frames: config.pre_skip_frames,
            codec_delay_ns: config.codec_delay_ns,
            seek_pre_roll_ns: config.seek_pre_roll_ns,
            packet_count: stats.packet_count,
            pcm_frame_count: stats.real_frames,
        },
    };
    let mut packets = Vec::new();
    let read = visit_vp9_opus_packets(
        WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 10,
            fps_denominator: 1,
        },
        &source,
        |packet| {
            packets.push(match packet {
                ParsedAvPacket::Video {
                    data,
                    timestamp_ns,
                    keyframe,
                } => {
                    format!("video:{timestamp_ns}:{keyframe}:{:x}", Sha256::digest(data))
                }
                ParsedAvPacket::Audio {
                    data,
                    timestamp_ns,
                    discard_padding_ns,
                } => {
                    format!(
                        "audio:{timestamp_ns}:{discard_padding_ns}:{:x}",
                        Sha256::digest(data)
                    )
                }
            });
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(read.video_frame_count, 1);
    assert_eq!(read.audio_packet_count, stats.packet_count);
    packets
}

#[test]
fn opus_ebml_boundary_real_track_uid_collisions_preserve_metadata_and_packets() {
    let (bytes, config, stats, padding) = real_media();
    // 原实现/修复对照可显式重放已经保留的真实 mux 字节；普通门禁仍现场生成。
    let bytes = if let Some(path) = std::env::var_os("CLIPPY_EBML_FIXTURE") {
        fs::read(path).unwrap()
    } else {
        bytes
    };
    retained("original.webm", &bytes);
    let original_packets = strict_packets(&bytes, &config, stats);
    // UID 的实际整数宽度由 writer 决定，只等长修改其数值；不移动索引或块。
    // 独立的保存媒体审计再按父路径确认这个偏移，并验证其它字节完全相同。
    let uid_header = bytes
        .windows(2)
        .position(|part| part == [0x73, 0xC5])
        .unwrap();
    let uid_size = usize::from(bytes[uid_header + 2] & 0x7F);
    assert!(bytes[uid_header + 2] & 0x80 != 0 && (4..=8).contains(&uid_size));
    let uid_start = uid_header + 3;
    let variants: [(&str, [u8; 8]); 4] = [
        ("uid-delay.webm", [0x56, 0xAA, 0, 1, 2, 3, 4, 5]),
        ("uid-preroll-none.webm", [0x56, 0xBB, 0, 1, 2, 3, 4, 5]),
        ("uid-preroll-value.webm", [0x56, 0xBB, 0x81, 1, 2, 3, 4, 5]),
        ("uid-padding.webm", [0x75, 0xA2, 0, 1, 2, 3, 4, 5]),
    ];
    let mut readings = Vec::new();
    for (name, uid) in variants {
        let mut modified = bytes.clone();
        modified[uid_start..uid_start + uid_size].copy_from_slice(&uid[..uid_size]);
        retained(name, &modified);
        assert_eq!(strict_packets(&modified, &config, stats), original_packets);
        let actual = (
            read_unsigned_element(&modified, DELAY),
            read_unsigned_element(&modified, PRE_ROLL),
            read_unsigned_element(&modified, &[0x2A, 0xD7, 0xB1]),
            read_signed_element(&modified, PADDING),
        );
        eprintln!(
            "EBML UID fixture {name}: offset={uid_start}, sha={:x}, actual={actual:?}",
            Sha256::digest(&modified)
        );
        readings.push((name, actual));
    }
    let expected = (
        Some(config.codec_delay_ns),
        Some(OPUS_SEEK_PRE_ROLL_NS),
        Some(WEBM_TIMECODE_SCALE_NS),
        Some(padding as i64),
    );
    assert!(
        readings.iter().all(|(_, actual)| *actual == expected),
        "expected={expected:?}, actual={readings:?}"
    );
}

#[test]
fn opus_ebml_boundary_opaque_void_and_blocks_are_not_fields() {
    let scale = &[0x2A, 0xD7, 0xB1];
    let fake = [DELAY, PRE_ROLL, scale, PADDING]
        .into_iter()
        .flat_map(|id| element(id, &[1]))
        .collect::<Vec<_>>();
    let mut body = element(&[0xEC], &fake);
    body.extend(element(
        &[0x15, 0x49, 0xA9, 0x66],
        &element(scale, &WEBM_TIMECODE_SCALE_NS.to_be_bytes()),
    ));
    let mut track = element(DELAY, &6_500_000_u64.to_be_bytes());
    track.extend(element(PRE_ROLL, &OPUS_SEEK_PRE_ROLL_NS.to_be_bytes()));
    // 去掉内层 Segment，使 Void 与 Tracks 成为同一个 Segment 的子节点。
    let tracks = track_document(&track);
    body.extend_from_slice(&tracks[12..]);
    let mut cluster = element(&[0xA3], &element(PADDING, &[1]));
    cluster.extend(element(&[0xA0], &element(PADDING, &[0xFE])));
    body.extend(element(CLUSTER, &cluster));
    let document = element(SEGMENT, &body);
    assert_eq!(
        (
            read_unsigned_element(&document, DELAY),
            read_unsigned_element(&document, PRE_ROLL),
            read_unsigned_element(&document, scale),
            read_signed_element(&document, PADDING)
        ),
        (
            Some(6_500_000),
            Some(OPUS_SEEK_PRE_ROLL_NS),
            Some(WEBM_TIMECODE_SCALE_NS),
            Some(-2)
        )
    );
    let wrong_parent = element(
        SEGMENT,
        &element(&[0x15, 0x49, 0xA9, 0x66], &element(PRE_ROLL, &[1])),
    );
    assert_eq!(read_unsigned_element(&wrong_parent, PRE_ROLL), None);
}

#[test]
fn opus_ebml_boundary_missing_fields_cannot_come_from_uid_payload() {
    for id in [DELAY, PRE_ROLL] {
        let fake = element(id, &[1]);
        let bytes = track_document(&element(&[0x73, 0xC5], &fake));
        assert_eq!(read_unsigned_element(&bytes, id), None);
    }
    let bytes = padding_document(&element(&[0xA1], &element(PADDING, &[1])));
    assert_eq!(read_signed_element(&bytes, PADDING), None);
    let scale = &[0x2A, 0xD7, 0xB1];
    let bytes = element(SEGMENT, &element(&[0xEC], &element(scale, &[1])));
    assert_eq!(read_unsigned_element(&bytes, scale), None);
}

#[test]
fn opus_ebml_boundary_child_cannot_escape_parent() {
    let mut entry = element(TRACK_ENTRY, &[]);
    entry.extend(element(PRE_ROLL, &OPUS_SEEK_PRE_ROLL_NS.to_be_bytes()));
    assert_eq!(
        read_unsigned_element(&element(SEGMENT, &element(TRACKS, &entry)), PRE_ROLL),
        None
    );
    let child = element(PRE_ROLL, &OPUS_SEEK_PRE_ROLL_NS.to_be_bytes());
    let mut truncated = child.clone();
    truncated.pop();
    assert_eq!(
        read_unsigned_element(&track_document(&truncated), PRE_ROLL),
        None
    );
}

#[test]
fn opus_ebml_boundary_invalid_vint_and_unknown_scalar_are_rejected() {
    for prefix in [
        vec![0],
        vec![0x56],
        vec![0x56, 0xBB, 0],
        vec![0x56, 0xBB, 0xFF],
        vec![0x56, 0xBB, 0x01, 0],
    ] {
        let mut children = prefix;
        children.extend(element(PRE_ROLL, &OPUS_SEEK_PRE_ROLL_NS.to_be_bytes()));
        assert_eq!(
            read_unsigned_element(&track_document(&children), PRE_ROLL),
            None
        );
    }
    // 标量自身无效，不能因为后面文件中有足够多字节而读作合法整数。
    let mut unknown = PRE_ROLL.to_vec();
    unknown.push(0xFF);
    unknown.extend([0; 127]);
    assert_eq!(
        read_unsigned_element(&track_document(&unknown), PRE_ROLL),
        None
    );
}

#[test]
fn opus_ebml_boundary_normal_signed_values_and_segment_sizes_are_preserved() {
    for data in [
        vec![0x7F],
        vec![0x80],
        vec![0xFF],
        i64::MIN.to_be_bytes().to_vec(),
        i64::MAX.to_be_bytes().to_vec(),
    ] {
        let mut value = 0_i64;
        for byte in &data {
            value = (value << 8) | i64::from(*byte);
        }
        let shift = (8 - data.len()) * 8;
        value = (value << shift) >> shift;
        let known = padding_document(&element(PADDING, &data));
        assert_eq!(read_signed_element(&known, PADDING), Some(value));
        let mut unknown_segment = known;
        unknown_segment[4..12].copy_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(read_signed_element(&unknown_segment, PADDING), Some(value));
    }
    let normal = track_document(&element(PRE_ROLL, &OPUS_SEEK_PRE_ROLL_NS.to_be_bytes()));
    assert_eq!(
        read_unsigned_element(&normal, PRE_ROLL),
        Some(OPUS_SEEK_PRE_ROLL_NS)
    );
}
