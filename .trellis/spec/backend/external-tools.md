# 外部工具集成规范

## 原则

- **优先 CLI 子进程**：通过 `std::process::Command` 调用系统工具（如 tesseract、ffmpeg），不使用 `-sys` crate 动态链接
- **避免 `-sys` crate 动态链接**：跨 Ubuntu 版本 SONAME 不兼容（如 `libtesseract.so.4` vs `.so.5`），会导致分发的 deb/AppImage 无法启动
- **唯一例外**：有 `bundled` feature 的 crate（如 `rusqlite = { features = ["bundled"] }`）将 C 库静态编译进二进制

## 集成模式

```rust
// 1. 运行时检测工具是否可用
pub fn is_available() -> bool {
    Command::new("tool")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

// 2. 调用工具，缺失时返回友好错误
pub fn process(input: &[u8]) -> Result<String, String> {
    let child = Command::new("tool")
        .args(["--flag"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "工具不可用：请安装 xxx".to_string()
            } else {
                format!("启动工具失败: {}", e)
            }
        })?;
    // ...
}
```

## 前端协作

- 后端提供 `tool_available` IPC 命令
- 前端在功能入口检查可用性，不可用时显示安装指引
- i18n 键约定：`action.{feature}Unavailable`

## CI 检查

- Release 阶段用 `readelf -d target/release/binary | grep NEEDED` 审计动态依赖
- 新增 `-sys` crate 时必须确认：是否有 bundled feature？是否引入了跨版本不兼容的 SONAME？

## deb 依赖

- 可选工具**不放** `depends`（避免安装失败）
- 通过应用内提示引导用户安装
