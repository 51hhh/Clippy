# WIN-NATIVE-01 / W62 — 录屏控制修复后的 Windows release 编译

源码 `75792220cea374dd2f1e6526122be30dad328e6a`；前置文档 `ffcad8d75f9d00fcddaa6c468413fdda3f080b2a`。本轮只补充最新代码的 release 编译与文件证据，
包含 W59 控制状态预检和 W61 `REC-VIDEO-CONTROL-FAILURE-01` 错误传播修复。
生产代码、原测试及期限保持；在 `codex/windows-control-release-validation` 记录验证。

## 构建与产物

默认及录屏 QA 分别使用新的独立 Cargo target；冻结同一个干净源码，随后恢复原分支。
两次 native child、包装器和终端实际退出码均为 0。Tauri CLI 执行
`build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，叠加 Windows/CI 配置，
Cargo `--locked --offline -vv`、npm offline、4 jobs。QA 加
`--features recording-windows-av-qa` 及冻结源码时生成的 app-local CRT 配置。
libvpx 构建脚本仍可下载固定 SHA-256 的源码归档；Cargo offline 不代表整个编译不联网。
依赖 warning 原样保留，编译结果不替代 W61 的严格 lint 与测试门禁。

| 变体 | EXE 字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `e8966cf4b46aace806795651f767279790fca236d2791b9b1c52405f4f4b6636` |
| 录屏 QA | 31709184 | `f5cb6397c3006abccedb1d57cf432cfcd3e4fd10541c6fed04e8c0c680e0d8b0` |

实际主程序 rustc 命令确认 opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
分别核对库和二进制 feature 指纹：默认不启用 recording-*；QA 含 WGC/WASAPI、VP9 源码
构建及 Opus/WebM。两个产物均为 AMD64 PE32+ / Windows GUI，嵌入证书目录为空。
release panic=abort 在进程 panic 终止时不执行 Drop；测试配置下的 unwind 清理不能替代它。

## 运行库文件核对

QA 的 10 份既有 release CRT 已核对 Microsoft 签名、版本、AMD64 PE 和哈希；来源写入
源码绑定的 provenance。对实际 EXE、相邻 DLL 和 licenses 调用原验证器，遍历直接与延迟
导入及递归 CRT 依赖。实际所需：`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
只做文件核对，没有加载 DLL、启动程序或验证缺少系统 CRT 的机器。
单独保存的 EXE 不含相邻 DLL；完整未打包目录在 QA RESULT 的 output.original。

## 分层证据与未完成项

证据：`src-tauri/target/current-release-default-7579222/`、
`src-tauri/target/current-release-qa-7579222/`，包括 RESULT、实际 PID/退出码、源码树和输入
哈希、逐行 UTF-8 编译日志、主程序参数、feature 指纹、PE 和 QA 部署核对。
独立审计保存为 CODE-VERIFICATION-AUDIT；原 830b12b release、早期 EXE、旧安装及
39 项桌面验收记录的字节保持。

W61 相同源码完整 Windows 门禁仍为 33 通过、0 失败、1 Linux smoke 跳过；
默认 Rust 1254 / QA Rust 1381 各 5 ignored，录屏领域 415，前端 81 文件/1403。
两张 Rust 图重叠，领域与新合同已含总数。本轮不重复测试，不把编译/文件比较加入通过数；
本地产品修复仍为 41。当前源码 release 的编译与文件检查已补充，实际运行仍未验证。

当前 SHA 跨平台/codec CI、其它宿主本地门禁、真实 WGC/WASAPI、长时同步、双轨顺序控制
时钟、桌面复测、安装器/updater、无 CRT 启动、Win10/多屏/混合 DPI 仍未验证。
W53 历史 AVI 超时和 W59 历史 Opus 失败根因仍未证明。桌面保持停止，未安装、打包或发布。
原 `REC-VIDEO-CONTROL-FAILURE-01` 最后一项复合验收保留未完成。
