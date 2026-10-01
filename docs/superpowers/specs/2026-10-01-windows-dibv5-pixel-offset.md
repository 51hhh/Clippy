# WIN-DIBV5-PIXEL-01 — Windows DIBV5 像素起始偏移

## Goal

修复 W22 已复现的 Chrome DIBV5 小图读取 UnexpectedEof，并阻止较长数据尾部将错误偏移掩盖成
成功的错图。独立分支 codex/windows-dibv5-pixel-offset，基于预算修复文档 6069cce。

## Requirements

1. 对 CF_DIBV5 的完整 V5 头计算真实像素偏移（头及颜色表）；不额外跳过头内已包含的颜色掩码。
   保留 Chrome BI_RGB/alpha 修正、上下行方向、Firefox 24-bit 以及现有像素/透明度断言。
2. 通过借用 DIB 字节的 BMP 文件视图向锁定解码器提供显式 bfOffBits，避免其无文件头推导错误；
   只合成 14 字节文件头，不复制整份剪贴板数据、不改依赖版本/锁文件或其它平台。
3. 文件视图的 Read/Seek 正确处理跨头读取、相对/末尾寻址、EOF 与非法负位置；
   尺寸预算仍在整图像素分配前执行。畸形 V5 头、颜色表越界返回 ConversionFailure。
4. 原 Chrome/Firefox 夹具和逐像素断言保留；另补顶向/底向、有尾部数据的 bitfields 与颜色表
   夹具，确认首个像素、顺序和 alpha。测试离线，不访问剪贴板、桌面或超大分配。
5. Windows 本机门禁和 Native CI 显式运行整个 image_data 原生解码组，保持预算组七项独立执行。
   红绿、完整本机门禁、远程 CI 和真实互操作分层记录，CHANGELOG、补丁与总计划同步。

## Acceptance Criteria

- [x] 原 Chrome 与额外 bitfields 用例在旧路径失败，原逐像素断言未经削弱地在修复后通过。
- [x] Firefox、透明度、顶/底向、颜色表及文件视图 Read/Seek 合同通过，超限图片仍前置拒绝。
- [x] 干净源码 SHA 的完整 Windows 默认/录屏 QA 本机门禁通过，CI 定向入口已接线。
- [x] 文档与补丁来源记录同步，真实互操作、新 SHA CI 等未验证边界明确。

## Out of Scope

修改 registry image crate、调整全部 BMP 文件导入、颜色管理、真实 Chrome/Firefox/Office
桌面互操作、其它平台图像读取、升级依赖、安装新包、合入 dev 或发布。

## Review Evidence

cf59157 原代码的 chrome_dibv5 测试失败（exit 101），预算修复后同一夹具也失败。
锁定 image 0.25.10 的 BmpDecoder::read_metadata 对 V5 bitfields 在头末尾额外跳过 12 字节；
new_without_file_header 将该位置用于像素读取，因此 5×5×4 字节夹具读到尾部不足。
使用有文件头解码器的显式 bfOffBits 可避免此错误推导，像素数据仍由相同解码器处理。
微软文档有关 V5 头内掩码和颜色表的描述，以及 arboard 原夹具均保留供复核；
文档对掩码外置的表述存在上下文差异，修复以 packed CF_DIBV5 与逐像素合同为边界。

- [BITMAPV5HEADER](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header)
- [Bitmap header types](https://learn.microsoft.com/en-us/windows/win32/gdi/bitmap-header-types)

## Verification

Windows MSVC 离线红基线 2 passed / 2 failed，exit 101：原 Chrome UnexpectedEof，
带尾部/优化颜色表的 bitfields 实际读出全 200 的错误像素；未修改原夹具或断言。
修复后五项 image_data、三项文件视图、七项预算与九项 CF_HTML 共 24 passed，exit 0；
arboard --lib --tests 严格 clippy 通过。红源码和红绿日志在主检出的 windows-dibv5-red。
Read/BufRead 视图只借用 DIB，头为内联 14 字节数组；不复制整图，Seek 允许超 EOF 但拒绝负位置。

源码 25fb5d7159af66a88d829eda199efa649698633d 在 Windows 11 x64 / PowerShell 5.1 /
Rust 1.98.1 MSVC / Node 24.21.0 的完整 scripts/ci-windows.ps1 -RecordingQa exit 0：
27 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1046 passed / 5 ignored，
QA Rust 1099 passed / 5 ignored；两图重叠不累加。前端 75 文件 / 1292 passed；Python 质量 33 /
视觉纯合同 3、check、严格 clippy、fmt、类型/静态检查、生产构建与构建入口通过。
四项 Windows 依赖测试入口分别实际执行 9/7/5/3 个合同，共 24 passed，未执行上游桌面测试。

验证前后源码检出干净，stdout/stderr 哈希已核对；证据位于主检出的
src-tauri/target/windows-dibv5-native-qa-25fb5d7/RESULT.json。
Windows Native CI 两个新入口的 job/条件和命令经 YAML 解析核对，但远程新 SHA CI 尚未运行。
jsdom Canvas 实现缺失的既有提示不计为真实像素渲染证据，未增加新依赖来掩盖该边界。
已安装 QA 包仍为 45769c9，不含本修复；桌面操作保持停止，真实提供者、Office、Windows 10、
多屏以及其它平台原生图仍未验。既有红/失败证据保留，不用本次通过改写旧 SHA 的结果。
