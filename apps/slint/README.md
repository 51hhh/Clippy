# Clippy Slint

Clippy v0.2.0 的原生 UI 迁移入口。固定 Slint 1.19.0，不依赖 Tauri 或 WebView；当前已通过
`clippy-platform` 解析与旧版一致的数据目录，并通过 `clippy-storage` 专用线程读取 SQLite/FTS 历史。
剪贴板监听、动作、托盘和其余完整产品功能尚未接入，也不生成正式发布包。
legacy v0.1.x 仍保留在 `apps/legacy-tauri/`。

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
cargo build --locked --release -p clippy-slint --bin clippy-slint-qa --features qa-build
cargo-packager --config apps/slint/Packager.qa.toml --formats deb,appimage
```

编译和打包是两个显式阶段；Packager 配置只描述产物，不通过 shell hook 启动 Cargo。这样 Windows
不会因扩展长度工作目录被 `cmd.exe` 当成 UNC 路径而丢失 workspace 根目录。

该配置不等于 v0.2 正式打包流程。数据目录已按生产/QA 身份隔离，单实例和快捷键仍要等对应平台服务
接入后再做运行期验收。Native QA workflow 在三平台分别生成 deb/AppImage、NSIS/MSI、app/DMG；Windows/macOS
是否成功仍以对应原生 runner 的同 SHA 结果为准。

桌面分发选用 Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0。
窗口内保留 `AboutSlint` 归属组件，QA 包同时携带完整的第三方许可文本。
