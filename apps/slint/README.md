# Clippy Slint

Clippy v0.2.0 的原生 UI 迁移入口。当前只包含可编译、可关闭并展示框架归属的壳层，固定
Slint 1.19.0，不依赖 Tauri 或 WebView；尚未接入完整产品功能，也不生成正式发布包。
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
cd apps/slint
cargo-packager --config Packager.qa.toml --formats deb
```

该配置不等于 v0.2 正式打包流程。数据目录、单实例和快捷键隔离要等对应平台服务接入后再做运行期
验收；Windows/macOS 包也必须由各自原生 runner 生成。

桌面分发选用 Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0。
窗口内保留 `AboutSlint` 归属组件，QA 包同时携带完整的第三方许可文本。
