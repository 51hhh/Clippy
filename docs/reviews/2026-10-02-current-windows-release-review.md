# WIN-NATIVE-01 / W58 — 当前 Windows release 编译与文件核对

生产源码 `830b12b43051efa7eebad36a3d1a4434975785b2`；构建前文档 `cd8a5e3fcca3b80f051a37fa88b15677cd9e528a`。
沿用 W57 已验证的生产/测试，不修改它们；本轮仅补充原生 release 编译证据。
默认与 QA 分别使用独立 Cargo target，冻结相同干净源码，完成后恢复原分支。
随后在 `codex/windows-current-release-validation` 记录本次验证，未增加修复或测试通过数。

## 实际构建

两次实际 native child、包装器和终端退出码均为 0。Tauri CLI 使用
`build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，叠加 Windows/CI
配置，Cargo `--locked --offline -vv`、npm offline、4 jobs；QA 另加
`--features recording-windows-av-qa` 与冻结源码后生成的 app-local CRT 配置。
libvpx build.rs 仍下载仓库固定 SHA-256 的源码归档；Cargo offline 不表示它不联网。
没有安装系统工具或运行应用。
现有 urlencoding/rxing 依赖的编译 warnings 保留于日志；native exit0 是编译结果，
不替代 W57 的严格 lint 门禁，未重新计算 lint 或测试通过数。

| 变体 | EXE 字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `5848fdb9dadfc4c8dcf2e68b797e94afd47647063f14a1dc6460f4b6a3aaa538` |
| 录屏 QA | 31708160 | `1c68377de98086ba6325c38628ba1898499fdffc0083e5d90c90af398d446508` |

两个 EXE 均为 AMD64 PE32+ / Windows GUI，嵌入证书目录为空。本轮未签名、未打包，
该目录为空不能作为发行信任结论。原默认 `f5ad5da` 与原 QA `1c66112` 文件哈希保持，
旧安装、证书和桌面验收记录未改。

## 编译选项与功能图

Cargo 配置及实际主程序 rustc 命令均确认 opt-level=s、panic=abort、lto=fat、
codegen-units=1、strip=symbols；分别保存主程序参数与库/二进制 feature 指纹。
默认图不启用 recording-*；QA 包含 WGC/WASAPI、VP9 源码构建与 Opus/WebM。
这是编译图证据，实际设备行为未验证。release panic=abort 不在进程 panic 终止时
执行 Drop；原 unwind 测试不能替代这一边界。

## CRT 与文件部署

QA 使用既有 MSVC 14.44.35207 工具链的 10 份 14.44.35211.0 release CRT，
逐份核对 Microsoft 签名、版本、
哈希和 AMD64 PE。冻结源码 `830b12b43051efa7eebad36a3d1a4434975785b2` 记入 provenance，保存其哈希。
本轮对实际输出 EXE、实际相邻 DLL 和 licenses provenance 调用原验证器，遍历
直接及延迟导入、递归 CRT 依赖，核对部署字节与来源。实际所需 CRT：
`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
未加载 DLL；文件闭包不证明无 CRT 系统启动或 Windows 10/设备兼容。

证据分别位于 `src-tauri/target/current-release-default-830b12b/` 和
`src-tauri/target/current-release-qa-830b12b/`。`RESULT.json` 保存 clean/source tree、
输入哈希、命令、实际 PID/退出码及 finally 恢复；`CODE-VERIFICATION-AUDIT.json`
独立复核已结束构建，`MAIN-RUSTC-COMMAND.json`、`PE-INSPECTION.json`、feature
指纹及 QA `PAYLOAD-VERIFICATION.json` 保存各层依据。stdout/stderr 为逐行 UTF-8
文本，并非管道逐字节快照。单独保存的 QA EXE 不包含相邻 DLL；完整未打包目录
见 QA RESULT 的 output.original。

## 验证边界

W57 的默认 Rust 1240 / QA Rust 1359（各 5 ignored）、前端 81 文件/1403，
以及完整 Windows 门禁 33 passed / 0 failed / 1 Linux smoke skipped 保持历史原数；
本轮不重复执行、不把两张 Rust 图相加，也不把编译或文件比较算作测试通过。
累计本地修复仍为 39。

当前 SHA 三宿主/codec CI、其它宿主本地门禁、实际 WGC/WASAPI/长时同步、
安装器/升级/卸载/updater、无 CRT 启动、Windows 10、混合 DPI/负坐标/多屏及
修复后桌面回归仍未验。桌面保持停止，未合入 dev 或发布。
W53 历史 AVI 用例一次 30 秒超时缺少 worker 原错，根因仍未知；当前编译成功
不改变该结论。继续按代码层面审查独立可复现缺陷，不以扩大测试期限掩盖失败。
