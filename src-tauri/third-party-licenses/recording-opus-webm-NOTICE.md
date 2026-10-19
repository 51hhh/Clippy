# Recording Opus/WebM prototype notices

The optional `recording-opus-webm` feature uses these additional components:

| Component | Version | License | Source |
|---|---|---|---|
| `opusic-c` | `1.6.1` | BSD-3-Clause | <https://github.com/DoumanAsh/opusic-c> |
| `opusic-sys` | `0.7.5` | BSD-3-Clause | <https://github.com/DoumanAsh/opusic-sys> |
| bundled libopus | `1.6.1` | BSD-3-Clause | <https://gitlab.xiph.org/xiph/opus> |
| patched `webm` / `webm-sys` wrapper | `2.2.1` | MPL-2.0 | <https://github.com/DiamondLovesYou/rust-webm> |
| bundled libwebm sources | crate `webm-sys 2.2.1` | BSD-3-Clause | <https://chromium.googlesource.com/webm/libwebm> |

`opusic-sys 0.7.5` carries the complete libopus 1.6.1 source tree in its crates.io package. Its
`bundled` feature builds that tree locally with CMake and does not fetch native code during the build.
The `opusic-sys` license file and the bundled libopus `COPYING` file are byte-identical, so Clippy
ships one full text covering both entries. The separate `opusic-c` BSD-3-Clause text is stored beside
this notice.

Clippy vendors `webm 2.2.1` and `webm-sys 2.2.1` only to expose libwebm's existing Opus codec delay,
seek pre-roll, and block discard-padding APIs. Patch manifests live in each vendor directory; the
wrapper and bundled libwebm license texts already shipped with the VP9 prototype remain applicable.

This feature is not part of the default product build and is not connected to the recording session
or user interface yet.
