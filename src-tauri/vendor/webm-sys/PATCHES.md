# Clippy patches for `webm-sys 2.2.1`

Upstream: <https://github.com/DiamondLovesYou/rust-webm>

The bundled libwebm already implements `Track::set_codec_delay`, `Track::set_seek_pre_roll`,
`SegmentInfo::set_timecode_scale`, and `Segment::AddFrameWithDiscardPadding`. Clippy exposes those
four operations through the crate's existing C ABI and rejects null/empty frame and codec-private
inputs at that boundary. No bundled libwebm source file is modified.
