# WIN-NATIVE-01 / W68 — QPC 精度修复后的 Windows release 验证

源码 `aac5e0a728d46adc7bd7603188f41b9380138650`；前置文档 `472d449d37c3bbc85a953e9c2cbbcdf393f807fa`；分支 `codex/windows-qpc-precision-release-validation`。
包含 `REC-WASAPI-QPC-PRECISION-01` 修复。生产代码、测试、原模块/期限保持，本轮只补充同源码的编译与文件证据。

## 构建和来源

默认与录屏QA使用两个新的独立Cargo target，冻结同一干净源码；实际native child、包装器、终端均退出0。
Tauri CLI `build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，Windows/CI配置，
Cargo `--locked --offline -vv`、npm offline、4 jobs；QA追加feature `recording-windows-av-qa` 与冻结源码准备的CRT配置。
绑定728份源码/配置输入及完整Git tree；两个图输入一致。
vendored libvpx可下载固定SHA-256源码归档，Cargo offline不能据此解释为全程断网。
逐行日志及第三方warning保留；编译不替代W67严格lint和测试门禁。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `17587e2ade5ff5ae28dedf8ce71e008297d48fb70baf31fafd39c8fcfd9fa676` |
| 录屏QA | 31714304 | `396564d852224c15ff878d37894373881a02c198ca72c94501df032f14b59e8d` |

实际主程序rustc参数：opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
两个图各自的库/二进制feature指纹核对：默认无recording-*；QA启用WGC/WASAPI、VP9源码构建与Opus/WebM。
EXE为AMD64 PE32+、Windows GUI，嵌入证书目录为空；未生成签名/安装器。默认录屏入口仍关闭。
release panic=abort在进程panic终止时不执行Drop；原unwind测试不证明这种终止方式的清理。

## QA运行库文件

既有10份release CRT的Microsoft签名、版本、AMD64 PE、哈希/来源与实际EXE、相邻DLL/licenses核对。
直接/延迟导入及递归CRT闭包验证，所需`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
只读文件，没有加载应用或DLL；未验证无系统CRT机器的启动。
保存的单个EXE不含相邻DLL；完整未打包目录由QA RESULT的output.original标明。

## 分层证据与未完成项

证据 `src-tauri/target/current-release-default-aac5e0a/`、`current-release-qa-aac5e0a/`：
PID/实际退出码、冻结输入、UTF-8日志、编译参数、feature、PE、运行库来源/部署/导入均保存。
W67同源码完整门禁复查33/0/1 Linux smoke skipped；默认1281/QA1425各5ignored，
前端81文件/1403、录屏领域459。17项新合同已含总数（15两图/2仅QA），跨图重叠不累加。
原API诊断4/0，精度新API绿色保护13；原完整合成双轨complete 40ms/4帧/1920有效PCM和真实codec/文件不证明设备同步。
本轮新修复/测试为0，44项本地修复/43项历史保持；构建、10份CRT和文件检查不计测试通过。
W67初版完整门禁32/1/1，QA1424/1/5：旧分段测试清单写入os error 5；同SHA隔离1/0和完整复查通过。
首次失败日志/退出码保留，原TempDir失败媒体未留，根因仍未明；本轮没有重跑或修改这项测试。
W53/W59/W63历史失败根因同样保持未明。既有release/已安装包/39项桌面记录字节保持。

当前SHA跨平台/codec CI、其它宿主、真实WGC/WASAPI/设备时钟漂移/长时同步、桌面、
安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验。桌面停止，无应用或设备启动、安装、推送、PR、合入或发布。
原规格最后复合验收仍未完成。

同步只读检查混音源的原生控制时刻和输出帧网格：候选保存在ignored `windows-mixed-audio-frame-contract-followup.json`。
未执行原Rust API诊断，未选择/修改控制或PCM策略，不计测试/修复；须后续确认并另定稳定合同。
