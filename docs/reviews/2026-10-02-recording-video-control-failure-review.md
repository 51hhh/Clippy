# WIN-NATIVE-01 / W61 — 视频源控制后 pipeline 失败关闭

需求 `REC-VIDEO-CONTROL-FAILURE-01`；基线 `4bf8ceeb909787eee0d5ef451fde8a084447dca3`；
分支 `codex/recording-video-control-failure`。规格在修改前建立。

## 复现与修复

视频源 pause/resume 已改变 running 状态后，pipeline 可能拒绝无效时间戳或返回
Aborted/Closed。旧 worker 将错误交给请求者却返回 Ok(None)，继续轮询或停在暂停等待。
六条原 worker 合同均显示 native语义 hooks=[pause]/[pause,resume]、running=false/true，
请求已有具体 pipeline 错误但 terminated=false。错误源是合成状态源，未运行真实 WGC。
原单轨MJPEG与QA双轨VP9/Opus owner 随后 Stop=Ok，实际清单为 complete，分别留下AVI与
WebM分段/最终文件，控制根因被覆盖。清单/媒体/长度/SHA与调用日志在断言前保留。

只在已经调用 source 后，把原 pipeline Result 克隆交给请求者，再用 ? 传播给采集线程。
旧 abort guard 中止开放pipeline、保留原终态与队列前缀，source在线程内析构。原 owner
停止/编码联动返回同一具体根错，两个会话的实际清单 interrupted、无 complete媒体或未提交partial。
首帧前暂停、重复暂停、未暂停恢复继续在 source前拒绝且可继续；正常控制与source错误保持。
没有改源API、pipeline/timeline规则、AV顺序控制/时钟、FPS、容量或原测试/期限。

## 验证

最终同字节八项原API夹具，旧实现0 passed / 8 failed，修复录屏领域415/0=原407+新8。
新7在默认/QA两图、新1双轨仅QA；领域已含这些测试，不能累加为新的总数。
六条控制故障均核对回复/原join错误、线程退出/单次析构、排队首帧与终态保留；
两个会话核对原owner根错、源回收及实际interrupted清单/无未提交文件。
首次新夹具漏掉MJPEG quality字段造成编译失败；日志保留，不算旧功能的失败用例。
修正夹具后才取得八条运行期失败。原完整worker测试模块逐字保持，仅增加新模块登记。

完整Windows门禁待冻结源码后执行。

## 保留边界

测试帧源/音源模拟原控制语义并注入故障，codec和文件是真I/O；不证明实际WGC/WASAPI故障
或实际UI自动清理。普通WGC时间戳是否倒退也未宣称。双轨顺序控制与长时同步仍需独立验证。
W53历史AVI30秒超时、W59历史Opus失败根因未证明。桌面停止；当前SHA CI、其它宿主门禁、
真实设备/长时同步、安装器/无CRT启动、Win10/多屏、当前release未验；旧830b12b产物保留。
证据在 `src-tauri/target/video-control-failure-contract/`。
