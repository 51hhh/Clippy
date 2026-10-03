# WIN-QA-MSI-PROVENANCE-01 — QA MSI 的运行库来源清单名称

## Goal

修复 Windows QA MSI 的来源清单落盘名称与生产部署验证器不一致的问题。关联
WIN-NATIVE-01 / W07、WIN-QA-CRT-01；基线源码 `8889192`，前置文档 `0e21f72`。
W85 使用原 release EXE 和既有工具生成并解包四个 unsigned 包，没有安装或启动。
QA MSI 实际 File 表和 cabinet 为 `licenses/PROVENANCE.json`；同源码 NSIS 为
`licenses/windows-qa-vc-runtime.json`。十份 CRT 字节均保持，但实际生产验证器对 MSI
部署模型 exit 1 / ENOENT，NSIS exit 0。此处是文件级部署合同失败，不是已观察到的
安装或 Windows loader 失败。

## Requirements

1. 准备器把 staging 清单本身命名为 `windows-qa-vc-runtime.json`，与资源目标 basename
   一致；继续在 `licenses/` 部署，保持 schema、源码身份、清单哈希和原十份 CRT。
2. 生产文件验证器要求 canonical staging 名称及原目标映射，拒绝旧 `PROVENANCE.json`
   映射；不放宽干净源码、签名、版本、架构、递归依赖、路径和部署字节校验。
3. 增加 canonical 接受、旧 basename 拒绝、Windows PowerShell 5/7 实际准备入口合同。
   原测试正文、超时及其它源码保持；共享夹具只适配新 canonical 文件名。
4. 新清单以当前干净源码准备；真实 NSIS/MSI 通过只读解包核对实际 File/目录/字节。
   若复用旧 EXE，必须记录原编译 SHA、全部 Rust/前端编译输入身份与资源准备 SHA，
   不将跨 SHA 复用写成新 SHA 的原生编译或完整门禁。
5. 原默认/QA 门禁、同 SHA CI、平台和 Native QA 分层记录，安装包构建不计测试通过数；
   原 8 Requirements / 9 AC / 47 Tasks 与未完成验收保留。

## Acceptance Criteria

- [x] 同一新增合同在旧实现失败、修复后通过；原用例及拒绝条件保留。
- [x] PowerShell 5/7 准备器及文件验证器对 canonical 文件名通过，旧名映射拒绝。
- [x] 实际 NSIS/MSI 解包后的十份 CRT、canonical 清单及包内 EXE 来源分别核对。
- [ ] 当前源码完整 Windows 门禁、同 SHA CI 和安装/无 CRT 系统启动完成。

## Out of Scope

不修改正式发布策略、基础许可证内容、Rust/前端功能、默认录屏开关或第三方 Tauri；
不安装/启动/升级/卸载应用、运行 WebView2 或设备采集，不修改系统/信任、不推送/合入/发布。
其它基础许可证的 MSI basename 差异保留记录，内容均核对，不能称为 canonical 重命名已修复。

## Primary references

[Tauri CLI 2.11.4 MSI](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/msi/mod.rs)
使用资源目标目录，但生成 File 时只有 Source，文件名取源 basename。使用 canonical 源文件名
避免依赖 MSI 的重命名行为；[同版本 NSIS](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)
保留原配置目标名称。以上代码推理与 W85 实际 File 表/解包结果互相核对。

## Verification

W85干净228cc93定向80/4→84/0，完整Windows33/0/1、前端81/1409、默认1341/QA1493各5ignored。
实际QA MSI/NSIS canonical清单和十份CRT文件校验均exit0；EXE复用8889192，未计新SHA release编译。
最后复合AC已有完整本机门禁证据，但同SHA CI和真实安装/启动未验，保持未完成。
详见 [W85审查](../../reviews/2026-10-04-windows-qa-msi-provenance-review.md)。
