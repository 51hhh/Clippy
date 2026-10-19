# WIN-CF-HTML-01 — Windows 富文本剪贴板片段边界

## Goal

关闭 Windows 富文本剪贴板读取中的未受缓冲区边界约束的片段复制路径，避免畸形 CF_HTML 偏移
导致越界读取或进程崩溃。这是 `WIN-NATIVE-01` / W05 的独立修复，基于 `46e3fdc`。

## Requirements

1. vendored arboard 的 Windows HTML 读取先取得实际剪贴板字节，再用安全切片取得片段；
   不再调用锁定 clipboard-win 5.4.1 的 `raw::get_html`，不改 Cargo 版本或其它平台实现。
2. 片段必须同时提供十进制 StartFragment / EndFragment；拒绝缺失、负数、非数字、溢出和倒序。
   索引必须在实际缓冲区内，且片段不能切断 UTF-8 字符；不使用裸指针或 unchecked 切片。
3. 保留 UTF-8 字节偏移、前导零、CRLF/LF/CR 头行、可选上下文及正常写入/读取的原样片段。
   正常 arboard wrap_html 的 Unicode 片段往返必须无额外换行或包装层。
4. 畸形输入返回 ConversionFailure，既有 watcher 的纯文本/图片回退保留；不打印剪贴板内容。
5. 离线测试复核旧偏移校验缺口和新安全解析器，不触碰系统剪贴板、不实际执行越界复制。
   本机 Windows 门禁和 Windows Native CI 显式执行依赖库的定向解析合同，防止默认 Cargo 成员遗漏。
6. 更新补丁来源记录、CHANGELOG、审查计划；与桌面互操作、其它平台和同 SHA CI 分层记证据。

## Acceptance Criteria

- [x] 旧偏移校验的越界输入回归失败，新解析器拒绝同一输入；生产路径接线合同通过。
- [x] Unicode 实际 wrap_html 往返、合法换行/零填充及畸形偏移/UTF-8 合同通过。
- [x] Windows 默认和录屏 QA 完整本机门禁通过，绑定源码 SHA；Windows CI 定向入口已添加。
- [x] 补丁记录、CHANGELOG 和总计划同步，未验证边界保留。

## Out of Scope

执行畸形数据的原生越界复制、写系统剪贴板或操控桌面、改变 HTML 渲染/XSS 清洗策略、
重写 Windows 全部剪贴板 API、图片解码和总分配预算；不安装新包、合入 dev 或发布。
本修复关闭 Clippy/arboard 的调用路径，不声称修复 registry 中 clipboard-win 本身或已证明可利用泄漏。

## Review Evidence

Cargo.lock 锁定 clipboard-win 5.4.1；其 raw.rs::get_html 只检查 end-start <= GlobalSize，
没有检查 end <= GlobalSize，随后 ptr::copy_nonoverlapping(data+start, ..., end-start)。
例如 start 在缓冲区外、差值仅 3 字节仍会通过。vendored arboard::Get::html 直接调用此函数，
Clippy 的 watcher 通过 arboard 读取；这是实际可达源码风险，尚未在桌面执行畸形复制。

微软 [CF_HTML 合同](https://learn.microsoft.com/en-us/windows/win32/dataxchg/html-clipboard-format)
规定偏移按 UTF-8 字节计算，允许零填充和 CRLF/LF/CR；上下文可省略，片段偏移仍需提供。

## Verification

Windows MSVC 离线红基线提取锁定依赖的旧范围校验，不调用其裸指针复制，也不访问系统剪贴板。
九项合同为 2 passed / 7 failed，exit 101；两项越界输入仍返回成功范围，生产接线仍使用旧函数。
修复后九项定向合同通过，exit 0；实际 arboard wrap_html 的中文/emoji/重音字符片段原样返回。
保留旧校验与结果的源码、日志及哈希在主检出的 `src-tauri/target/windows-cf-html-red/`。

修复源码 `50b7778ec9e4bd52fa31aa657be607877c4990ef`，Windows 11 x64、Windows PowerShell 5.1、
Rust 1.98.1 MSVC、Node 24.21.0；完整 `scripts/ci-windows.ps1 -RecordingQa` exit 0，
24 passed / 0 failed / 1 skipped（Linux smoke），包含实际执行的 CF_HTML 九项解析回归。
验证前后 Git 检出干净，stdout/stderr 哈希已核对。

| 层级 | 结果 |
|---|---|
| 本机默认 Rust | 1046 passed / 5 ignored；check、严格 clippy、fmt 通过。 |
| 本机录屏 QA Rust | 1099 passed / 5 ignored；check、严格 clippy 通过。与默认图重叠，不累加。 |
| Windows arboard 解析 | 9 passed；仅离线字节及生产接线，不执行系统剪贴板 API。 |
| 本机前端 | 75 文件 / 1292 passed；类型、静态合同、供应链、生产构建及产物入口通过。 |
| Windows CI 配置 | Native Check 的 Windows job 已加入定向组；YAML 解析和 job 条件已核对，尚未远程执行。 |
| 同 SHA 原生 CI / 桌面 | 未运行；当前安装包仍为旧源码 45769c9。 |

独立证据位于主检出的 `src-tauri/target/windows-cf-html-native-qa-50b7778/RESULT.json`。
本规格、CHANGELOG、总计划与审查记录为后继文档更新；测试绑定源码 50b7778。
真实富文本提供者互操作、Windows 10、其他平台原生图、总分配预算和新 SHA CI 保留未验证。
未执行原生畸形复制，不声称已观察到崩溃或可利用的内存泄漏。
