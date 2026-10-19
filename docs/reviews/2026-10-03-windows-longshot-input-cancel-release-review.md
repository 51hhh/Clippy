# WIN-NATIVE-01 — 取消输入修复的Windows release验证

W82；关联WIN-LONGSHOT-INPUT-CANCEL-01，[规范](../superpowers/specs/2026-10-03-longshot-input-cancel.md)。
修复源码`00f40cc5cf8b3cf3c332dc7cce6be47cd07aefcc`；前置文档`81c04b2b88dcca82bc88966ab27193b5dd9abdcb`；分支`codex/windows-longshot-input-cancel-release-validation`。
默认和显式录屏QA release从同一干净源码编译完成；本轮没有新增产品修复或测试。

## 有限源码复核

复核输入许可、controller取消/owner、恢复、测试诊断cfg、release配置及构建入口九份文件。
另核对manager：auto_append在claim成功后才进入with_scroll，匹配InFlight让finish返回Busy；
finish先消费会话时，已取得的owner克隆仍需manager claim，Empty拒绝后不会进入滚动。
这是有限代码顺序推理，不是原生调度或输入复现。
取消先撤销共享许可和manager lease，只等待已进入的同步mutation；焦点poll/settle/抓帧不在锁内。
Windows移动、激活、滚轮、迟到恢复使用同一许可；不能撤回已进入OS队列的事件或保证API不阻塞。
锁定Enigo0.6.1 Windows构造/scroll/Drop来源与W81一致；scroll不登记held键，该调用链无额外析构键输入。
test_artifacts仅cfg(test)，release panic=abort不运行unwind Drop，测试目录保留不证明进程强杀恢复。
这些是有限源文件检查，未确认新缺陷，不表示全项目或其它平台/真实原生输入已经验收。

## 编译与来源

全新独立default/qa Cargo target；同一干净冻结Git tree、751份原始编译输入和完整门禁1072份原始输入绑定。
Cargo.lock/vendor不归一化；HTML/CSS/资源同时保留。两组源输入完全一致，构建后恢复原干净文档HEAD。
Tauri build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc及Windows/CI配置；
Cargo --locked --offline -vv、npm offline、4jobs。QA显式recording-windows-av-qa及同源码CRT资源配置。
Cargo离线不保证vendored libvpx不获取固定哈希源码归档；没有新增工具安装或系统更改。
默认终端70140、QA终端26116：实际native子进程、包装器和终端均exit0，均已终结。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28748288 | `609276ebc4b14f825295abd7ccddecaf65ae3f8de0c29c3e0b5c789ffe7b4bdd` |
| 录屏QA | 31718912 | `fecfe7ba23676251e5760f240e1cca0a4c172b2d7cbb251b3a51ed2ec0a5088f` |

核对真实主程序rustc参数opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols，库/二进制feature指纹。
默认无recording-*；QA为WGC/WASAPI、固定来源VP9、Opus/WebM。默认产品录屏入口继续关闭。
QA基准二进制只编译，未运行或采集，也不计测试通过。
两份EXE为AMD64 PE32+ Windows GUI，证书目录为空；PE/功能指纹是文件证据，不是运行结果。

## 依赖与计数

默认直接/延迟DLL名称集合与前置d05cd3e默认文件相同，未引入QA私有VC++名称；没有比较导入函数或启动程序。
QA10份既有Microsoft CRT的签名/版本/AMD64/来源/哈希/license、相邻部署和导入闭包验证；实际需要msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll。
保存的独立EXE不包含相邻DLL，完整原始目录见QA RESULT.output.original；未打包、签名、安装或加载DLL。
日志、rustc命令、fingerprint、PE与原始CRT清单保存。PROGRESS未退出值仅历史采样，终态取RESULT/实际终端。

W81同源码完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped保持：默认1334/QA1486各5ignored，
前端81文件/1405，QA录屏516已含1486；4项新回归已含总数。已通过且源码未变，不重复测试。
本轮新增修复/用例/通过数0，50产品修复/49历史与W80测试诊断分别保留；构建、PE与CRT检查不计测试通过。
证据：current-release-default-00f40cc、current-release-qa-00f40cc、current-release-closure-00f40cc及isolated-release-00f40cc。

## 未完成边界

当前SHA七项CI未验，本轮未查询或推送/触发远端；旧d05/其它SHA结果保持历史身份。
真实取消/接管/焦点/输入/DPI/权限、WGC/WASAPI/设备拔插/漂移/长时同步、其它宿主、Win10/多屏混合DPI/负坐标、
无开发CRT环境启动、安装升级卸载/updater仍未验。已安装旧457包的39项桌面记录保持2 passed/1 failed/36 not_run，
旧包不含后续50项修复。本轮保持桌面暂停，没有应用/设备控制、安装、证书/系统更改、PR、合入或发布。
W53/W59/W63/W67原失败根因仍未明，原8R/9AC/47任务及末两项全局AC、此修复末项复合AC保持未完成。
准备读取冻结源码中不存在的后置文档时发生只读错误，改用git show读取81c文档；不是native构建/测试失败。
一次可选编译进程名称查询因部分名称不存在返回1，实际构建仍运行；该只读诊断退出不计native结果。
WIN-NATIVE-01全局目标保持未完成。
