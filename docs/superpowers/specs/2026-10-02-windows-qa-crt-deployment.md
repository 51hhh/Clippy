# WIN-QA-CRT-01 — Windows 录屏 QA 的 C++ 运行库部署

## Goal

修复 Windows 录屏 QA 包依赖构建机已安装 C++ 运行库的交付缺口。对应 WIN-NATIVE-01 / W48，
基线 `162f5e8`，保持录屏非默认 feature；QA 应携带验证过的 x64 release CRT 文件。

## Requirements

1. W47 冻结 `f5ad5da` 的 QA PE 导入 `MSVCP140.dll`，默认 PE 没有；旧 `45769c9` MSI
   保存清单与原 resources 配置都没有该运行库。以真实 PE/保存文件和原配置建立缺失基线，
   不声称已在无运行库机器上观察到启动失败。
2. 从当前 Visual Studio 安装的 `VC/Redist/MSVC/<version>/x64/Microsoft.VC143.CRT`
   取允许再分发的 release DLL，拒绝 onecore、debug、其它架构与未知文件。每份文件必须
   为 AMD64、Microsoft 有效 Authenticode 签名，完整文件版本不得早于所选 MSVC toolset；复制前后
   SHA-256 相同。记录源码 SHA、工具链、文件版本、签名与哈希，不下载或安装全局运行库。
3. 生成仅供显式 Windows QA 使用的 Tauri resources 覆盖配置，将 CRT 放在 EXE 同目录；
   原许可证资源保持。QA 使用显式 x64 target 子目录，避免 MSI 扫描共享默认 release
   目录中的 DLL。默认 feature、默认 Windows 配置、正式 release workflow 保持。
4. 文件验证器解析 PE 的 direct/delay imports，并校验 CRT 的递归依赖闭包、配置根目录
   映射、受信 staging 清单哈希、每份文件哈希与架构。缺文件、路径别名、旧清单、篡改、
   非 OS/未提供 DLL 均拒绝；不能用构建机 System32 或全局已装 CRT 填补应用本地缺口。
5. Windows QA workflow 在构建前准备配置，在构建后验证真实 EXE/同目录 CRT，再上传
   源码与运行库证明。PowerShell 默认/QA 门禁与前端测试覆盖准备/拒绝合同；生产 Node
   解析器不加载 DLL，PowerShell 签名查询不改变证书或信任。
6. 同一组受控文件/PE/部署合同覆盖缺根依赖、缺递归依赖、delay import、架构错误、
   不合法 RVA/截断、资源位置错误、清单/文件篡改、旧源码与错误 publisher/version。
   原用例及产品源码保留，Windows 完整门禁和独立 unbundled QA 构建分层记录。
7. 应用本地 CRT 的更新由每次 QA 重建部署负责；微软推荐正式产品采用可独立更新的
   redistributable 中央部署，本轮不改变正式 release 策略。安装、无 CRT 系统的真实启动、
   当前 SHA CI、多屏、Windows 10、其它平台与 Wayland 保留未验。

## Acceptance Criteria

- [x] 原 QA PE 与原资源清单的缺失对照已记录，默认图不引入 CRT。
- [x] 同目录资源、x64/版本/有效 Microsoft 签名、原始 DLL 字节及完整闭包验证通过。
- [x] 受控拒绝/PE 合同、真实 PowerShell 入口、原用例保留核对通过。
- [x] 干净源码完整 Windows 默认/QA 门禁，源码/日志/文档与同 ID CHANGELOG 同步。
- [x] 显式 target 的 unbundled QA 构建与实际产物/同目录 DLL 核对通过；原默认产物保持。
- [ ] 同 SHA 远程 CI、NSIS/MSI 安装和无 CRT 的 Windows 10/11 真实启动通过。

## Out of Scope

不安装、启动、卸载应用或系统运行库，不修改证书/信任，不恢复桌面操作，不下载新系统
工具、不执行 Linux/WSL，不推送/创建 PR/合入/发布。不把静态依赖闭包当作实际 Windows
loader、WinRT/设备、安装器或旧系统兼容性证明，不改变 Rust 的 panic/CRT 链接策略。

## Primary references

- [Microsoft C++ 二进制兼容限制](https://learn.microsoft.com/en-us/cpp/porting/binary-compat-2015-2017?view=msvc-170)：
  运行库不得早于组件使用的最新 toolset；校验完整文件版本，不能只比较主、次版本。

- [Microsoft DLL 部署](https://learn.microsoft.com/en-us/cpp/windows/deployment-in-visual-cpp?view=msvc-170)：
  应用本地 DLL 须与 EXE 同目录，更新由应用部署负责。
- [可再分发 DLL 与版本](https://learn.microsoft.com/en-us/cpp/windows/determining-which-dlls-to-redistribute?view=msvc-170)：
  使用 Visual Studio release redist 文件与兼容 toolset；UCRT 属 Windows 10+ 系统组件。
- [Tauri CLI 2.11.4 NSIS](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)
  与 [MSI](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/msi/mod.rs)：
  两者读取显式 resources；MSI 另收输出根目录 DLL，不能借此假设 SDK CRT 自动部署。

## Verification

新合同 43 项通过，含 PowerShell 5/7 的受控 publisher 元数据与实际配置发布入口；原门禁
13 项原正文未改，共 56 passed。首次新增脚本为 35 passed / 1 failed，原因是无 BOM 中文
注释在 PowerShell 5 被错误解码，随后补 BOM 与当前 shell 模块选择，并增加 shell 回归。
这不是旧产品的红基线；旧 QA 的缺依赖来自真实保存 PE 与原 MSI/配置的静态对照，
`windows-qa-crt-contract/ORIGINAL-DEPLOYMENT-AUDIT.json`。默认/QA direct/delay 表均解析，
QA 缺 MSVCP140，默认 OS-only 闭包通过，旧 MSI 表无 CRT。没有 DLL 加载或真实启动。
完整 Windows 门禁、真实 SDK 与隔离 target 的实际构建进行中；W47 与 29 项历史身份保留。

真实 SDK 十份 DLL 已在 `a6bd01d` 通过标准目录、Microsoft 有效签名、14.44.35211.0、
AMD64、原字节哈希及递归闭包。该 SHA 首次完整门禁为 32 passed / 1 failed / 1 skipped；
Rust 默认 1193/QA 1256（各 5 ignored），前端 80 文件中 79 passed / 1 failed，1365 passed /
1 failed。失败是原 `regression-guards` 对连续 feature 参数的字符串合同；只调整新 target
参数顺序，原测试正文不改，定向回归通过。首次门禁不计整轮通过，新 SHA 完整重跑待核对。

`3ad9df9` 完整 Windows 门禁 33/0/1、前端 80/1366、默认 Rust 1193/QA 1256（各
5 ignored）通过。隔离 target 的 unsigned/unbundled QA build native/wrapper/terminal 0；
十份真实 SDK DLL 与十七份基础许可证、递归闭包、主 rustc 参数、默认 EXE 保持均核对。
这些文件实际版本 14.44.35211.0 新于 toolset 14.44.35207；产物未启动/安装。
复核发现旧检查谓词只比较主、次版本，受控 14.44.10000.0 元数据被接受。
新增同补丁拒绝/同版本及新 minor 接受合同：原检查器 47 passed / 3 failed，分别为 Node、
PowerShell 5/7 旧补丁被接受；修正完整版本比较后，同五十项通过。受控 publisher 元数据
不代表实际旧 DLL 已部署或签名造假；当前真实 SDK 未修改。最终源码完整门禁/编译待核对，
前一阶段的成功和失败记录都保留各自 SHA，不互相替代。

最终干净 `5d900cadc0f3c1509e680b9fcf5c5f850b76a364` 完整门禁 native child/terminal 0，
33 passed / 0 failed / 1 Linux smoke skipped；默认 Rust 1193/QA 1256，各 5 ignored、
不累加，前端 80 文件 / 1373 passed，新五十项在内。最终隔离 x64 target 的
unsigned/unbundled QA 编译 native/wrapper/terminal 0，3m45s；十份实际 CRT、十七份
基础许可证、direct/delay/递归依赖与清单、默认 EXE 保持核对。原七十九份前端测试
文件原字节相同，原产品/Rust/vendor/锁文件/基础配置保持，比较和文件数不算测试通过。
源码/日志/工具输入与主 rustc profile/feature 参数、退出码和干净状态已审计。
最终 EXE SHA-256：`9e397a731466e33c86a0a713605e9537504327fed72ef07c4ceea1f6598e2109`。
当前 SDK 14.44/v143 文件部署已验；其它 SDK 布局、当前 SHA CI、安装器/无 CRT 系统启动、
Windows 10/多屏/录屏设备/其它宿主/Wayland 保持未验证；未启动/安装/推送/合入/发布。
详情见 `docs/reviews/2026-10-02-windows-qa-runtime-review.md`。原 29 项修复与旧真实 QA
身份保留；本需求的代码/文件/构建阶段完成，最后一项原生交付验收未完成。

W85补充：真实QA MSI清单basename与部署路径不一致，修复见WIN-QA-MSI-PROVENANCE-01。
当前228cc93完整Windows门禁33/0/1、前端81/1409；新QA MSI/NSIS canonical清单与十份CRT
文件验证通过，EXE编译身份仍为8889192。原最后安装/无CRT启动/同SHA CI复合AC未完成。
详见 [W85审查](../../reviews/2026-10-04-windows-qa-msi-provenance-review.md)。
