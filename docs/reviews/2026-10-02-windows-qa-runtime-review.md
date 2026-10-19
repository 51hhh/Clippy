# WIN-NATIVE-01 / W48 — Windows 录屏 QA 运行库部署审查

需求 `WIN-QA-CRT-01`；基线 `162f5e8`，独立分支 `codex/windows-qa-runtime`。
最终被测源码 `5d900cadc0f3c1509e680b9fcf5c5f850b76a364`；规格见
[`2026-10-02-windows-qa-crt-deployment.md`](../superpowers/specs/2026-10-02-windows-qa-crt-deployment.md)。
本轮按用户指令仅代码/原生 CLI/保存文件，未恢复桌面、安装或运行应用。

## 问题与修复

W47 保存 QA PE 导入 MSVCP140.dll，而默认 PE 不依赖该 DLL；旧 `45769c9` MSI 的保存
文件表与原配置没有部署运行库。新解析器对真实保存 direct/delay 表的检查拒绝 QA 的
缺失依赖，默认 OS-only 闭包通过；这不是在无运行库机器上观察到的启动失败。

从本机 Visual Studio 2022 / MSVC 14.44.35207 标准 desktop x64 release redist 准备十份
DLL。检查 Microsoft 有效 Authenticode 签名、完整版本、AMD64、原字节哈希和递归依赖，
拒绝 onecore/debug/未知文件、缺失或篡改。生成带源码 SHA 和独立清单摘要的 QA resources
覆盖配置；显式 x64 target 隔离默认 release，构建后检查真实同目录 payload，workflow
上传运行库 provenance。基础许可证、默认 feature/Windows 配置与正式 release workflow 保持。

只比较主、次版本会接受受控 14.44.10000.0；改为完整版本不早于 toolset，同版本和新 minor
可接受。微软要求运行库至少与组件所用最新 toolset 同样新；本机实际文件为 14.44.35211.0。
应用本地 CRT 随 QA 重建更新，正式 release 的中央部署策略未改变。
([Microsoft 二进制兼容限制](https://learn.microsoft.com/en-us/cpp/porting/binary-compat-2015-2017?view=msvc-170)、
[DLL 部署](https://learn.microsoft.com/en-us/cpp/windows/deployment-in-visual-cpp?view=msvc-170))

已核对锁定 Tauri CLI 2.11.4：MSI 显式资源与根目录 DLL 会按已加入文件名去重；NSIS 使用
显式资源。有效已签名 DLL 的 signing 路径不重复签名。该静态检查不计安装器验证通过。
([MSI 源码](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/msi/mod.rs)、
[signing 源码](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/sign.rs))

## 分层验证

- 新文件/PE/PowerShell 合同最终五十项：旧版本谓词 47 passed / 3 failed，修改生产检查后
  同组五十项通过。publisher 元数据受控，不代表实际旧 DLL 被部署或签名伪造。
- 最初新增脚本 35/1 是 PowerShell 5 无 BOM 中文注释导致赋值被吞；补 BOM、选择当前
  shell 模块及 PowerShell 5/7 回归后通过。原 `a6bd01d` 完整门禁为 32/1/1，原连续 feature
  参数字符串合同失败；只调整新增 target 位置，原断言保留，`3ad9df9` 完整门禁通过。
- 最终干净 `5d900ca` 完整 Windows 默认/QA 门禁 native child/terminal exit 0：
  **33 passed / 0 failed / 1 Linux smoke skipped**。默认 Rust 1193、QA 1256，各 5 ignored，
  两图不累加；前端 **80 文件 / 1373 passed**，新五十项在总数内。Python 33 + 3、独立
  剪贴板 31/WGC 十八及严格 lint/供应链/生产构建入口保持通过。
- 原七十九份前端测试文件 Git blob 原字节相同，Rust/前端产品源码、vendor、锁文件与基础
  资源配置未改；比较数不增加测试通过数。
- 最终显式 `x86_64-pc-windows-msvc` unsigned/unbundled QA release 编译
  **native/wrapper/terminal exit 0**，3m45s；实际 direct/delay/递归依赖闭包、十份 DLL、
  provenance 和十七份基础许可证全部哈希核对。默认 release EXE 与其根目录无 CRT 保持；
  实际主 rustc 的 release panic=abort、fat LTO、单 codegen 和五个 recording feature 核对。
  构建、平台及文件数量都不算测试通过数；release 终止清理边界仍见 W47。

源码/输出日志/checked helper/真实子进程退出及干净检出已审计。前一 `3ad9df9` 编译与审计
保持历史身份；后续构建复用隔离 target 路径，旧复制 EXE 和不可变 staging 仍保留。

## Windows 基线与未完成项

已读 Windows 10 22H2 QA 基线和源码：窗口排除按 build 19041 门控；WGC 可选 border/cursor
设置错误不致命。微软声明 cursor 属性从 19041、border 属性从 20348 可用，排除值也从
Windows 10 2004 支持。该代码核对不证明 Win10 的窗口、授权、设备或实际边框行为。
([cursor 属性](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled?view=winrt-26100)、
[border 属性](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired?view=winrt-26100)、
[窗口排除](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity))

LLVM 只读导出表确认实际 MSVCP140 提供保存 QA 的 std::_Xlength_error 导入锚点；这不是
全部 OS API/export 或 loader 兼容性证明。其它 SDK 布局、当前 SHA 远程 CI、NSIS/MSI、
无预装 CRT 的 Win10/11 实际启动、多屏、录屏/音频、升级、其它宿主完整门禁与 Wayland
仍未验。没有推送/PR/合入/发布，历史真实 QA 仍 2 pass / 1 fail / 36 not_run，未安装新修复。

## 证据

- `src-tauri/target/windows-qa-crt-contract/ORIGINAL-DEPLOYMENT-AUDIT.json`
- `src-tauri/target/windows-qa-crt-contract/VERSION-PREDICATE-AUDIT.json`
- `src-tauri/target/windows-qa-crt-native-qa-5d900ca/RESULT.json`
- `src-tauri/target/windows-qa-crt-release-build-5d900ca/RESULT.json`
- `src-tauri/target/windows-qa-crt-release-build-5d900ca/SOURCE-AND-PAYLOAD-AUDIT.json`
