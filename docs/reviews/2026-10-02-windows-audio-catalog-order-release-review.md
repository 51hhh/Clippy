# WIN-NATIVE-01 / W78 — 音频目录查询顺序源码的 Windows release 验证

源码 `d05cd3e478934722273a33fb88c841648aeb1ef5`，前置文档 `e54d4fd57a4b2dbc05a8b82bbf66995e4ed41ef1`，分支 `codex/windows-audio-catalog-order-release-validation`。
包含 REC-AUDIO-CATALOG-ORDER-01 与48项前置修复；本轮源码/测试/新修复0。

## 编译与只读审查

只读核对设备目录、IPC宿主、lifecycle、冻结选区、平台转发、WASAPI和覆盖层七个文件，未确认新缺陷。
目录占位发生在原生枚举/await之前；只有当前、未发布且未过期的查询可发布。待发布目录与已发布目录共用容量，旧完成不能插入或复活目录。
设备选择在冻结截图候选消费前解析，原生ID只留在Rust并进入显式Windows source plan；GetDevice失败不改用系统默认设备。
Windows endpoint ID/属性转换释放原生分配，错误只返回静态操作名与HRESULT。前端初始/重试的generation保护保持。
此审查只支持上述代码路径，不证明全部项目、原生IPC、设备拔插或真实桌面。

两个新的独立Cargo target从同一干净冻结源码完成默认/非默认QA release编译，native/包装器/实际终端exit0。
绑定完整Git tree、743份相同编译输入，并比对同源码完整门禁的全部1064份原始输入，包含HTML/CSS/资源；Cargo.lock/vendor不做LF归一化。
Tauri build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc，Windows/CI配置；Cargo --locked --offline -vv、npm offline、4 jobs。
QA显式recording-windows-av-qa及同源码CRT配置；vendored libvpx固定哈希源码归档可获取，Cargo offline不等于全程断网，未下载安装器或系统工具。
全部第三方warning和日志保留。QA基准二进制只编译，未运行、采集或计为测试。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28747776 | `214f9fd0c8b0166e99c3e2d3a38bfb075acd3ed16ab861aa32d424b41a77df53` |
| 录屏QA | 31718400 | `96fc0acd184d5fa22ad8055d9fddc6f4307595cbb8cdbdb8fbea2fad3e98f3d6` |

主程序真实rustc参数opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols；库/二进制feature指纹各自核对。
默认不含recording-*，QA启用WGC/WASAPI、VP9源码构建与Opus/WebM；默认录屏入口仍关闭。
两份EXE均AMD64 PE32+ Windows GUI、证书目录为空。release panic=abort不执行unwind Drop，测试profile不能替代该终止方式的恢复验收。

## 文件依赖与验证层次

默认直接/延迟DLL名称集合与前置2bb12f9默认PE一致，未增加QA私有VC++导入；没有证明Win10或缺开发环境的启动。
QA既有10份Microsoft CRT签名/版本/AMD64/哈希/来源、相邻部署和license、直接/延迟/递归导入闭包核对；所需msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll。
文件验证未启动应用或加载其运行库进行运行验证。单个保存EXE不包含相邻DLL；完整目录见QA RESULT.output.original，不视为签名或可发布安装包。
证据在current-release-default-d05cd3e、current-release-qa-d05cd3e、current-release-closure-d05cd3e及isolated-release-d05cd3e。
PROGRESS中的未退出值只是历史采样，最终以RESULT及实际终端为准；构建/文件检查不计新通过数。

W77同源码完整Windows门禁33/0/1 Linux smoke skipped：默认1324/QA1475各5ignored、前端81/1405保持。
新10唯一用例已含总数（8 Rust两图/2前端），两图重叠不累加；QA中的509项recording用例已含1475，无额外领域重跑。
旧API受控完成顺序0/1，新API目录合同16/0、前端定向63/0；旧诊断与新接口回归分层，原完整模块/期限保持。
本轮不重跑已通过且未变更的测试；49项本机修复/48历史保持。已安装457包、39项桌面记录2 passed/1 failed/36 not_run与前置release保留原身份。

## 未完成边界

当前d05cd3e提交GitHub API422不可取得；同调用已知45769c9正控成功，仅证明查询访问可用，不证明其它SHA或所有CI状态。
未取得当前SHA三native/四原型CI成功，也未推送或触发CI。
其它宿主/Wayland、原生WGC/WASAPI及默认/非默认设备/拔插/漂移/长时同步/桌面、安装升级卸载/updater、无开发CRT启动、Win10/多屏/混合DPI/负坐标仍未验。
W53/W59/W63/W67原失败根因仍未明；没有应用、设备、桌面操作，也没有安装、签名、证书/新增工具、推送/PR/合入/发布。
最后复合AC和全局WIN-NATIVE-01目标保持未完成，W74的47项全局inventory不冒称当前49项全局重审。
