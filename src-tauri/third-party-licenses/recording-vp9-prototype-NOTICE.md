# Recording VP9 prototype notices

The optional `recording-vp9-prototype` feature uses these components:

| Component | Version | License | Source |
|---|---|---|---|
| `shiguredo_libvpx` | `2026.2.0-canary.1` | Apache-2.0 | <https://github.com/shiguredo/libvpx-rs> |
| libvpx | `v1.16.0` | BSD-3-Clause | <https://github.com/webmproject/libvpx> |
| `webm` / `webm-sys` Rust wrapper | `2.2.1` | MPL-2.0 | <https://github.com/DiamondLovesYou/rust-webm> |
| bundled libwebm sources | crate `webm-sys 2.2.1` | BSD-3-Clause | <https://chromium.googlesource.com/webm/libwebm> |

The corresponding full license texts are stored beside this notice. The upstream `webm 2.2.1` crate package
also ships the bundled libwebm BSD text as `LICENSE.TXT`; Clippy includes both the wrapper's declared MPL-2.0
license and the bundled C++ library's BSD-3-Clause text.

This feature is not part of the default product build. Its vendored Rust binding patch and pinned prebuilt archive
hashes are documented in `vendor/shiguredo_libvpx/PATCHES.md`.
