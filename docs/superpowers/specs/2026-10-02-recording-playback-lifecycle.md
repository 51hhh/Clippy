# REC-PLAYBACK-LIFECYCLE-01 — 播放准备响应的组件生命周期

## Goal

补齐 PX-REC-PLAYBACK-01 / Requirements 4–5：组件卸载、服务生命周期更换或用户关闭预览后，
迟到的播放准备响应必须释放不透明租约，不能重开播放器或修改后继请求的忙碌/错误状态。
对应 WIN-NATIVE-01 / W41，基线 092c413。原生产组件与延迟 Promise 已复现候选。

## Requirements

1. 每次准备绑定当前组件生命周期与请求身份；只有仍拥有该身份的响应可转换媒体 URL、
   更新播放器、错误或忙碌状态。卸载与服务生命周期清理立即使旧身份失效。
2. 迟到的成功响应通过发起请求的原 services 释放一次；释放拒绝须处理，不能产生未处理错误。
   迟到的失败响应不能覆盖新请求状态。服务重置后新请求可以开始，不被旧忙碌状态阻塞。
3. 关闭预览也撤销正在切换的播放准备；迟到结果不能重新打开预览。只清理该播放请求的忙碌
   状态，不解除仍在执行的导出/其它动作的忙碌状态。已显示租约继续按原协议释放。
4. 保持正常准备、失败重试、媒体 URL 失败释放、单播放器、切换/删除/合并和 API/IPC 合同；
   不修改 Rust 媒体协议、权限、默认 QA 门控、UI 文案或依赖。
5. 原 App.tsx 保持原字节跑生产组件/jsdom 红基线；同组测试修复后通过，旧组件/API 用例
   保持原字节。服务返回是受控替身，不能记作真实 Tauri 租约撤销或 WebView 解码证据。
6. 干净 SHA 完整 Windows 默认/QA 门禁与源/日志核对、同 ID 文档同步；两图重叠不累加。
   实际窗口/播放、其它宿主、同 SHA CI/安装更新仍保留未验。

## Acceptance Criteria

- [x] 原生产组件卸载后不释放迟到租约/关闭后重新打开的问题有红基线。
- [x] 卸载成功/释放拒绝、服务更换旧成功/旧失败、关闭中的切换响应正确结算。
- [x] 当前失败与 URL 转换错误保持可重试，关闭预览不会解除导出 busy。
- [x] 同组新测试与旧组件/API 原字节核对，源码/依赖/后台协议边界保持。
- [x] 干净源码完整 Windows 门禁与同 ID 文档/证据同步，真实桌面与其它平台留未验。

## Out of Scope

桌面操控停止，不运行应用/浏览器/播放器、不安装、不推送/PR/合入/发布。不将 jsdom
称作真机窗口或录屏证据。不扩展为其它动作 worker 的全生命周期整改，不改变结果窗
原生销毁/重建的识别策略、后端已开始的媒体响应或跨进程文件访问。

## Verification

原 App.tsx 保持原始字节，生产组件/jsdom 红基线 27 passed / 5 failed（旧组件 17、API 7
全部通过，新八项为 3 passed / 5 failed），native Node/终端 exit 1。三项观察到释放调用为
零，一项关闭后实际重新渲染了合成 video/source；第五项在新请求入口 disabled 断言处失败，
未执行到旧拒绝，不声称红基线已观察到新请求被旧拒绝覆盖。不是实际 Tauri invoke/播放。

pendingPlayback 身份绑定当前请求；卸载/服务清理或 closePlayback 使身份失效，成功迟到
通过旧 services 调用 release 并处理拒绝，current catch/finally 才可更新错误/busy。服务
新生命周期重置播放状态与 busy；关闭预览仅清理匹配的播放 busy，正在导出的 busy 保持。
同八项原字节修复后通过，连同旧 24 项共 32 passed / 0 failed（3 文件），Node/终端 exit 0，
没有未处理错误；旧测试 2 文件原字节、11 份关联文件保持。去除限定生命周期插入/guard
后 App 全部原正文保持，未更改 UI 文案、API/IPC、Rust 媒体/窗口协议或依赖。
服务更换用例是注入组件合同，不等于真实窗口重建；释放 API 调用不证明后端实际撤销。

干净源码 ba26a83ffc8b1db2519f76bae2d5aa1a6387c8af 完整 Windows 默认/QA 门禁确认原生
child 与终端 exit 0：30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust 1165、QA Rust
1228（各 5 ignored，重叠不累加），无新增 Rust 用例；旧媒体十三项、导出身份七项各图和
既有九条 manifest/导出/合并按原图通过。前端 78 文件 / 1315 passed，新八项在前端总数内；
Python 33 + 3，独立 vendor 十八项/剪贴板二十四项通过，check、严格 lint、供应链、构建/
入口通过。全量 Vitest 只打印简略汇总；同一默认配置发现清单 78 文件 / 1315 项含新八项
与旧 24 项，退出 0，结合原始测试/源码哈希及定向 32 全绿核对，未重跑测试主体。初次解析
器错误期望文件名的失败保留为证据解析记录，不计作门禁/产品测试失败，当前状态更新前已更正。
两份源码与门禁/checked helper/原始日志哈希、门禁前后干净检出核对；门禁后只改四份
Markdown，生产/测试字节保持；保存实际 QA/全未运行模板原字节保持，未安装更新包。
完整证据 C:\win\Clippy\src-tauri\target\recording-playback-lifecycle-native-qa-ba26a83；
原 App 红基线/原合同/测试发现/后续 ready 源码 C:\win\Clippy\src-tauri\target\recording-playback-lifecycle-contract。
真实窗口/播放/释放、其它宿主和当前 SHA CI 留未验，桌面操控停止。加载后 ready 的迟到
回调/StrictMode effect 退休留为下一步生产组件候选，尚未复现，不声称实际焦点发生变化。
