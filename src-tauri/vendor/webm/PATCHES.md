# Clippy patches for `webm 2.2.1`

Upstream: <https://github.com/DiamondLovesYou/rust-webm>

Clippy adds only the Rust surface required for standards-compliant Opus-in-WebM output:

- `SegmentBuilder::set_audio_codec_delay`;
- `SegmentBuilder::set_audio_seek_pre_roll`;
- `SegmentBuilder::set_timecode_scale`;
- `Segment::add_frame_with_discard_padding`.

The underlying behavior remains implemented by the bundled libwebm. No container element is
hand-written in Rust, and the existing video-only API is unchanged.
