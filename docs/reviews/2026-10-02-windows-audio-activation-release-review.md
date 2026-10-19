# WIN-NATIVE-01 / W66 — 音频激活边界修复后的 Windows release 编译

源码 `56d750c8b482108b15ebb5dd2787e418bdceecb6`；前置文档 `9164f08c0780c00339e5d5af8dc62070e13ae4d8`；分支 `codex/windows-audio-activation-release-validation`。
包含需求 `REC-AUDIO-ACTIVATION-BOUNDARY-01` 的音频恢复包含边界与 Windows 控制时钟修复。
本轮补充同源码的默认/录屏 QA release 编译和文件验证；产品、测试、旧期限保持。

## 构建与产物

两图分别使用新独立 Cargo target，冻结同一干净源码，实际终端、native child、包装器均退出0。
Tauri CLI 使用 `build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，
Windows/CI配置，Cargo `--locked --offline -vv`、npm offline、4 jobs；QA额外使用
`--features recording-windows-av-qa` 与在固定源码上准备的app-local CRT配置。
vendored libvpx可下载固定SHA-256源码归档，Cargo offline不代表全程断网。
第三方依赖warning和逐行编译日志保存；编译不代替严格lint或原测试门禁。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `98c236747d4a4d9ba167527b6daa6006c8d9d675cfad89c32d2c898d3b07d548` |
| 录屏QA | 31714304 | `23cc71c875f66386b7b9e38658587a1fd3115d84782f55fab46664910890febe` |

实际主程序rustc参数为opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
库/二进制feature指纹分别核对：默认无recording-*；QA启用WGC/WASAPI、VP9源码构建、
Opus/WebM。默认录屏入口仍关闭。产物均为AMD64 PE32+、Windows GUI，嵌入证书目录为空。
release panic=abort在进程panic终止时不执行Drop；原unwind测试不证明此终止方式的清理。

## QA运行库文件

既有10份release CRT的Microsoft签名、版本、AMD64 PE、哈希和源码provenance核对。
验证实际EXE、相邻DLL/licenses与直接/延迟导入、递归CRT闭包，所需
`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
文件检查未加载生成的应用或QA DLL，没有验证无系统CRT机器。
单独保存的EXE不含相邻DLL；完整未打包目录由QA RESULT的output.original标明。

## 分层证据与未完成项

证据：`src-tauri/target/current-release-default-56d750c/`、
`src-tauri/target/current-release-qa-56d750c/`，保存实际PID/退出码、源码树/输入哈希、
UTF-8编译日志、主程序参数、feature指纹、PE与QA部署文件核对。
W65同源码完整Windows门禁33/0/1 Linux smoke skipped；默认1266/QA1408各5ignored，
录屏领域442，前端81文件/1403。两图重叠，领域/新合同已含总数。
原七项API夹具2/5到7/0，另六新API绿色保护仍只证明合成source与真实codec/文件合同。
本轮新修复/测试为0，43项本地修复保持；构建、10份运行库和文件检查不计测试通过。
历史003fe2d/W62/W58 release、早期EXE、已安装包与39项桌面记录原字节保持。

当前SHA跨平台/codec CI、其它宿主、真实WGC/WASAPI、长时同步、桌面、安装器/updater、
无CRT启动、Win10/多屏/混合DPI仍未验。W53/W59/W63历史失败根因保持未明；
W65新增夹具重复定义的编译错误已修正，原失败日志保持，不能据此归因历史运行期失败。
桌面停止，无安装、打包、应用或设备启动、推送、合入或发布；原规格最后复合验收仍未完成。
