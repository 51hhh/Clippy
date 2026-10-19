# WIN-NATIVE-01 — 当前源码Windows安装包文件核对

2026-10-04，W87；原任务W07，关联[WIN-QA-MSI-PROVENANCE-01](../superpowers/specs/2026-10-04-windows-qa-msi-provenance.md)。
源码与EXE编译均`228cc935bc0767ed9906f03ff5680cc89e72febe`，前置文档`154752d0033abdf4e6197602cea08be7053e5fc1`；分支`codex/windows-current-package-file-validation`。

## 当前包和来源

使用W86当前源码默认/显式录屏QA release EXE及QA实际十份CRT，生成四个新包。
原Tauri CLI 2.11.4、NSIS/WiX现有662缓存文件及固定NSIS插件SHA保持，没有安装工具。
每组打包冻结干净228cc93，资源/图标/1073门禁原输入/752编译原输入核对，结束恢复154752d。
默认实际终端44561/子PID34316，QA终端7909/PID48232；native/包装器/实际终端均0且终结。
包生成新鲜，原EXE字节已恢复；下载WebView2仅为安装器动作，本轮没有执行。

| 变体 | 格式 | 字节数 | SHA-256 |
| --- | --- | ---: | --- |
| default | msi | 20078592 | `e249ab4f58d80833a235117adb356676172d48db498af1183a5a64670f99bbd2` |
| default | nsis | 15913022 | `4917f91e944a89b0f3ef2d458ff5099fdce00dfc501656bbfacf3a9a82db5067` |
| qa | msi | 21938176 | `b2bfd64fceb44fa53e5051bcfa35b63e2d1f2d72fd029c2840dc666151fa619f` |
| qa | nsis | 17276138 | `edd1e680fb73e96f8b20f15a7c7303fdedbeb41e2f915da9a49f3b65186470ee` |

实际Get-AuthenticodeSignature读取四个包均NotSigned；没有签名/系统信任操作。
旧W85修复包仍为资源228cc93/编译8889192，本轮新四包明确编译和资源均228cc93，旧身份不改。

## 文件核对

MSI使用OpenDatabase(path,0)固定只读表查询，并由WiX dark解出cabinet；NSIS使用7-Zip解包。
未调用安装API、运行installer/uninstaller或应用。两组只读/解包进程均实际exit0且终结，
总文件审计终端41655实际exit0。每组dark保留7条DARK1059 UI警告；File/Directory表直接读取，
据表建立实际部署模型，没有重命名模型文件，不将UI反编译称为完整通过。

四个包内EXE的全部字节等于对应当前release EXE，仅唯一Tauri格式标记UNK→MSI/NSS的
三字节变化；原程序其余字节不变。默认资源17份、QA资源28份，实际内容均核对。
MSI的三个基础许可证basename差异与W85默认包完全一致、内容保持；未称为重命名修复。
QA NSIS/MSI均部署licenses/windows-qa-vc-runtime.json，旧PROVENANCE.json不存在。
十份CRT及canonical清单哈希与当前准备记录一致，生产递归PE/direct/delay文件验证器两包均0。
这证明文件部署合同，不证明DLL加载、Windows API、启动、设备或安装升级卸载行为。

## 原门禁和剩余范围

本轮新增修复/源码/测试/通过数均0；52代码/部署修复、51历史保持。
原同源码完整Windows33/0/1、默认1341/QA1493各5ignored、前端81/1409不重跑或增计；
84定向合同及新增4项已含原1409。构建、平台矩阵、签名读取与解包不计测试通过数。
辅助字段投影曾将completionInventory字符串当对象而TypeError，已按字符串读回；
无产品/门禁/打包失败或重跑，原读取诊断与日志保留。
记录检查还发现W86六份提交文档经Git切换由CRLF转为LF；当前字节逐份等于154752d的
Git blob，逐份CRLF重建精确匹配W86历史原始哈希，两种字节记录及首次失败脚本保留。
此处仅限六份构建后Markdown，源码/测试/门禁/CRT/EXE原始哈希无归一化或放宽。
本轮新文档按仓库LF保存，提交后另存实际原始字节及Git对象身份供后续历史核对。

W85/W86、本轮包、原失败媒体/日志和已保留历史证据均保持原身份。同SHA CI仍not_run；
未查询/推送/workflow。Linux完整本地门禁/Wayland回归、其它宿主、Win10/多屏混合DPI/负坐标、
权限目标、WGC/WASAPI真实采集/声音/暂停恢复/设备默认路由/长时同步、无CRT启动、
安装升级卸载/updater/WebView2运行继续未验。旧457包39桌面项2/1/36不含后续52修复，桌面停止。
历史W53/W59/W63/W67失败根因仍未明，不能用本轮文件通过替代。
原8R/9AC/47Tasks和末两条全局AC、canonical/QA CRT最后复合AC继续未完成。
无桌面/设备/DLL加载、安装、系统/信任变更、PR/合入/发布；全局WIN-NATIVE-01未完成。

证据：主仓库src-tauri/target/current-package-228cc93；完整包和生成目录见PACKAGE-AUDIT。
这是未签名的本地文件检查产物，不是已安装或发布的QA交付。
