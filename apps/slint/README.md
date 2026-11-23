# Clippy Slint

Clippy v0.2.0 的原生 UI 迁移入口。当前只包含可编译、可关闭并展示框架归属的壳层，固定
Slint 1.19.0，不依赖 Tauri 或 WebView；尚未接入完整产品功能，也不生成正式安装包。
legacy v0.1.x 仍保留在 `apps/legacy-tauri/`。

```bash
cargo check -p clippy-slint
cargo test -p clippy-slint
cargo run -p clippy-slint
```

桌面分发选用 Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0。
窗口内保留 `AboutSlint` 归属组件；正式打包时还需把许可说明纳入第三方许可清单。
