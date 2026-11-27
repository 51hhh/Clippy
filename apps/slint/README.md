# Clippy Slint

Clippy v0.2.0 采用 Slint 1.19.0 和 FemtoVG。托盘驻留，使用 SQLite/FTS 历史，
支持原生截图、编辑、贴图工作区、手动长截图、OCR、翻译及区域录屏。窗口、OCR 和媒体
资源按需创建。v0.1.x Tauri 维护线位于 `apps/legacy-tauri/`。

```bash
cargo check -p clippy-slint
cargo test -p clippy-slint
cargo run -p clippy-slint --bin clippy-slint
cargo run -p clippy-slint --bin clippy-slint-qa --features qa-build
```

生产入口固定使用现有平台身份。QA 入口是独立的 `clippy-slint-qa` 可执行文件，使用
`com.clippy.qa.slint` 安装身份；共享 UI 和应用装配只保留一份。Linux QA 包可用固定版本的
`cargo-packager` 验证：

```bash
cargo install cargo-packager --version 0.11.8 --locked
cargo build --locked --release -p clippy-slint --bin clippy-slint-qa \
  --features qa-build,recording-playback-native-qa
cargo build --locked --release --manifest-path apps/ocr-worker/Cargo.toml --target-dir target
python3 -B scripts/prepare-slint-ocr.py --worker target/release/clippy-ocr-worker
cargo-packager --config apps/slint/Packager.qa.toml --formats deb,appimage
```

Linux 构建机用 Python 3 准备模型和运行库，用 `patchelf` 固定 AppImage 与 deb 共用的加载路径。
Linux QA 回放构建需安装 `libpulse-dev`。deb 声明 `libpulse0` 依赖，AppImage 包含 `libpulse.so.0`
及对应的运行库。两种格式的有声回放都连接系统 PulseAudio 或 PipeWire-Pulse 服务。
Packager 配置直接读取已经构建的二进制和资源目录。Windows 的 Rust 编译同样在打包前执行，避免
shell hook 在扩展长度工作目录下把路径解析为 UNC。

QA 与正式版本有独立的数据目录、单实例和快捷键身份。Linux 使用 GNOME Mutter/PipeWire 或 X11
截图。安装包自带 PP-OCRv6 small 和 CPU 运行库，打开图片文字工具时按需识别，运行时无需 Python。
显式配置的增强引擎继续生效，直接从源码运行时仍可用本机 Tesseract。QA 包使用原生录屏播放器，
正式包的录屏回放和文本发音依赖本机 `ffplay`。
真实平台验收以同一 SHA 上的 [Slint 真机 QA 记录](../../docs/native-qa.md#slint-v02-安装包验收)为准。

Native QA 工作流可在 Linux x64 选择 `linux_renderer_ab`。对照包的 `QA-BUILD.txt` 标记
`renderer=femtovg+skia-software`。启动同一个 QA 二进制时可通过 `SLINT_BACKEND` 选择
`winit-femtovg` 或 `winit-skia-software`。

正式打包读取 `Packager.toml`，由 `scripts/slint-release.mjs` 保留旧版身份和二进制名，
并生成各系统的安装版本。Linux 使用 Debian 的 `~rc.1` 等排序规则。Windows 安装版本第三段
为 `patch * 100 + rank`，alpha、beta、rc 和正式版的 rank 依次增加。macOS 保留普通版本号，
构建号使用同一排序。包外的 `BUILD-INFO-*.json` 记录真实 SemVer、平台身份、OCR 运行库来源和产物哈希。
`OCR-BUNDLE.json` 记录签名前的资源摘要。macOS 打包后另外检查 worker 和运行库的签名。
正式入口首次启动在数据目录的 `before-slint-v0.2` 备份配置和现有 SQLite。正式发布草稿保留
三平台 QA 及旧版 updater 升级的验收空间。

桌面分发选用 Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0。
窗口内保留 `AboutSlint` 归属组件，QA 包同时携带完整的第三方许可文本。
