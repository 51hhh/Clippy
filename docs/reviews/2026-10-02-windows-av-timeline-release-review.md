# WIN-NATIVE-01 / W64 — 双轨公共暂停修复后的 Windows release 编译

源码 `003fe2dc642627b9c6a071bf629be89342973941`；前置文档 `b3d8a47eed1a7e2ab567759dcc990771a34b963c`；分支 `codex/windows-av-timeline-release-validation`。
包含 `REC-AV-CONTROL-TIMELINE-01` 的公共暂停扣时与零交集边界修复。
本轮补充同源码默认/录屏 QA release 编译和文件证据，产品/测试未改。

## 构建与产物

两图各使用新独立 Cargo target，冻结同一个干净源码，结束后恢复文档分支。
两次 native child、包装器和实际终端退出码均为0；Tauri CLI 执行
`build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，Windows/CI配置，
Cargo `--locked --offline -vv`、npm offline、4 jobs；QA额外使用
`--features recording-windows-av-qa` 及固定源码时生成的app-local CRT配置。
vendored libvpx构建脚本可下载固定SHA-256源码归档，Cargo offline不代表全程断网。
编译日志和依赖warning保持，编译不替代严格lint或原测试门禁。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `c39a4bff925bb929e10c5fcf7dcb8cd80d1acb1b2d659efe3b0a4e5ee35c51fe` |
| 录屏QA | 31713280 | `dad9c38656851cd36c417ce15a9da637488b6c24afe3d061e203c8e1395ba6d8` |

实际主程序rustc命令：opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
库/二进制feature指纹均核对：默认不启用recording-*；QA启用WGC/WASAPI、VP9源码构建、
Opus/WebM。产物均为AMD64 PE32+、Windows GUI，嵌入证书目录为空。
release panic=abort在进程panic终止时不执行Drop；原unwind清理测试不能替代该边界。

## QA运行库文件

既有10份release CRT的Microsoft签名、版本、AMD64 PE、哈希和源码provenance核对。
原验证器核对实际EXE、相邻DLL/licenses及直接/延迟导入和递归CRT依赖，实际所需
`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
文件检查没有加载DLL、启动程序或验证无系统CRT机器。单独保存的EXE不带相邻DLL；
完整未打包目录由QA RESULT的output.original标明。

## 分层证据与未完成项

证据：`src-tauri/target/current-release-default-003fe2d/`、
`src-tauri/target/current-release-qa-003fe2d/`；包括实际PID/退出码、源码树/输入哈希、
逐行UTF-8编译日志、主程序参数、feature指纹、PE与QA部署检查。
同源码W63完整Windows门禁33/0/1 Linux smoke skipped；默认1254/QA1395各5ignored，
录屏领域429，前端81文件/1403。两图重叠，领域/新合同已含总数。
本轮没有新测试或产品修复，42项本地修复保持；编译/文件检查不计测试通过。
W62/W58 release、早期EXE、安装与旧39项桌面记录原字节保持。

当前SHA跨平台/codec CI、其它宿主、真实WGC/WASAPI、长时同步、桌面、安装器/updater、
无CRT启动、Win10/多屏/混合DPI仍未验。公共暂停合同使用合成源，真实codec/文件不能
代替真实设备同步。W53 AVI超时、W59 Opus失败、W63初版两项旧分段5秒超时根因未明；
W63同SHA隔离4/0与最终门禁通过不能证明当时原因。原日志与期限保持。
桌面停止，无安装、打包、推送或发布；原规格最后一项复合验收保留未完成。
