# PX-REC-PLAYBACK-01 — 录屏结果库内预览与播放

日期：2026-09-21

关联路线：`PX-REC-01`

## Goal

让用户不必先导出文件，就能在录屏结果库内确认完整录屏或异常恢复分段的内容，同时保持真实文件路径、
恢复清单和大文件字节流位于 Rust 信任边界内。

## Requirements

1. 只有 WebM 产物显示“播放”入口；AVI 诊断产物继续提供导出和在文件夹中显示，不能伪装成 WebView
   一定支持的媒体。
2. 播放前由后端重新解析会话与产物不透明 ID，并在阻塞线程中校验普通文件、清单长度和 SHA-256；
   WebView 不接收应用数据目录、文件路径、清单内容或可拼接文件名。
3. 校验通过后签发仅录屏结果窗可消费的有界播放租约。自定义 `recording-media` 协议只接受该窗口的
   `GET`/`HEAD`，支持单一 HTTP byte range，并把每次响应限制在 2 MiB，避免整段录屏进入 Rust/JS/Blob
   的多份内存副本。
4. 播放租约最多保留 8 个；切换预览、关闭预览、删除会话和关闭结果窗都会撤销对应租约。文件长度或
   修改时间在签发后变化时，后续请求必须失败。
5. 结果库同一时间只展示一个播放器。播放器首个解码帧同时作为按需缩略预览，不自动播放；加载或解码
   失败时保留导出与“在文件夹中显示”出口。
6. 播放入口不能扩大录屏结果窗现有 IPC 权限，也不能让其他窗口枚举或读取录屏产物。

## Acceptance Criteria

- [x] 完整 WebM 与中断会话的已提交 WebM 分段都能通过不透明 ID 打开同一个内嵌播放器。
- [x] 播放准备拒绝活动会话、未知产物、损坏清单、符号链接、大小或 SHA-256 不一致的文件。
- [x] 协议测试覆盖完整响应、`HEAD`、起止/开放/后缀 range、2 MiB 分块、越界和多 range 拒绝。
- [x] 未知、过期或其他窗口的租约不能读取媒体；签发后文件变化会使租约失效。
- [x] 前端测试覆盖播放、切换、关闭、解码错误、删除当前会话和 AVI 无播放入口。
- [x] IPC 合同、Rust check/clippy/test、前端测试、TypeScript、Vite 和完整本地门禁通过。

## Out of Scope

- 不合并独立恢复分段，不重编码或修改 WebM，也不加入剪辑、倍速、截图导出或自动播放。
- 不在本阶段加入系统音频、麦克风或开放 Windows、macOS、Wayland 产品入口。
- 不用 WebView 解码成功替代系统播放器、光标、混合 DPI、长时间资源与三平台真机验收。

## Verification

- `./scripts/ci-local.sh`：25 项通过、0 项失败、2 项按配置跳过；Rust 1034 项通过，前端 73 个
  文件共 1224 项通过，X11 录屏与剪贴板隔离 smoke、DOM/Xvfb、Canvas 像素、主窗口布局和 Vite
  构建均通过。
- `cargo check/clippy --features recording-vp9-prototype` 通过；媒体协议 5 项定向测试和录屏结果库
  前端/API 11 项定向测试通过；IPC 合同为 150 definitions、150 handlers、148 frontend commands、
  11 restricted window groups。
- 开发审阅页使用生产结果库组件和本地合成 WebM 验证 780px/420px 布局、播放器展开、首帧、关闭与
  租约释放；这项浏览器证据不等于 `recording-media` 自定义协议、系统 WebView 解码或三平台原生真机
  验收。
