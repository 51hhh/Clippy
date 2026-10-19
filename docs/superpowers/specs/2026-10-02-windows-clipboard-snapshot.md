# WIN-CLIP-SNAPSHOT-01 — Windows 富文本与替代文本的一致快照

## Goal

Windows 剪贴板 HTML 和替代文本来自同一次受保护读取，避免两次打开之间的外部复制混配。
对应 WIN-NATIVE-01 / W44，基线 43b400c；保留现有 HTML/text/image 优先级和失败回退。
受控原协议已复现快照混配；不声称真实 Windows 并发复制已观察。

## Requirements

1. 有效非空 HTML 与替代文本在同一个 OpenClipboard guard 下读取；guard 覆盖两种格式，
   成功、错误和展开时均释放。不能为读取替代文本重新打开剪贴板。
2. HTML 不存在、无效或为空仍回退普通文本、再回退图片；替代文本读取失败仍从本次 HTML
   派生纯文本；成功的空替代文本须保留，不误作失败。CF_HTML 原字节/UTF-8 边界继续校验。
3. Windows 单锁实现不依赖序列号可用或增长时点，避免零值和延迟渲染被当作一致性证明。
   不增加跨格式的全局长锁、循环重试或新 Windows 权限/SDK feature；原本单格式 API 保持。
4. 其它平台原顺序读取、Watcher WriteEpoch/抑制、去重/重试、存储、敏感标记与事件保持。
   用户复制内容不进入测试；回调与临时 SQLite 是离线证据，不替代系统/桌面验收。
5. 先提取原快照决策协议并保留原调用顺序建立红基线；同组读 adapter 测试修复后通过。
   另以生产 guard 协议的受控 Drop/回调测试核对锁生命周期；原解析/ watcher 合同保留。
6. 干净 SHA 完整 Windows 默认/QA 门禁与当前已有 CF_HTML 测试入口覆盖新增 guard 回归；
   两 Rust 图谱重叠不累加，vendor 回归单列。同 ID 文档/日志/源码同步，跳过和未验保留。

## Acceptance Criteria

- [x] 原顺序协议在受控外部复制后实际构造混配 HTML/text，首次失败断言和范围保留。
- [x] Windows paired 读取 adapter 保持同一份内容，受控临时 SQLite 未存入混配结果。
- [x] HTML/文本错误、空值、图片回退与原抑制/重试/去重合同保持。
- [x] 生产 guard 协议在双读取中持有、错误/空 HTML/展开时释放，替代文本失败与空值保持。
- [x] 旧解析/ watcher 正文与其它平台原协议核对，干净源码完整 Windows 门禁及文档同步。

## Out of Scope

不读取/写入真实剪贴板，不启动 watcher、应用、浏览器或操作桌面。没有安装、Linux/WSL、
推送/PR/合入/发布。第三方程序自己的延迟渲染内容质量、其它宿主编译与 Wayland/真机
并发复制保留未验，不把模拟 guard 或临时数据库当作真实 OpenClipboard 操作。

## Primary references

[OpenClipboard](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard)
打开期间禁止其它应用修改；成功打开后应关闭。
[GetClipboardSequenceNumber](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclipboardsequencenumber)
权限不足返回零，延迟渲染影响增长时点。当前方案直接沿用 Windows 原 guard，不据此观察系统。

## Verification

Windows HTML/text 共用一个 OpenClipboard guard，原单格式解码与接口保持；guard 覆盖双读取，
错误/展开由原 RAII Drop 释放。不依赖 sequence 零值或延迟增长；缺失/空 HTML 回退文本/图片，
替代文本失败派生本次 HTML、成功空值保持。其它平台默认顺序协议、Watcher 代次/抑制/
去重/重试及 CF_HTML 边界不变。提取原决策仅替换 reader adapter，初次 MSVC 红 27/4，
旧 23 项通过；两个混配、一次误用剥离文本与临时 SQLite 保存新文本/旧 HTML 实际复现。
新增抑制夹具的 clone 被严格 lint 拒绝，改引用切片，原断言/reader fixture 不变；初稿证据
保留。原提取 watcher 字节和原 native 单格式合同重放修正夹具仍 27/4；同修正八项原字节
修复后通过，共 31 passed。生产 guard helper 七项加旧 parser 九项共 16 passed，vendor
严格 lint 通过。guard/sequence 为受控模型，不是实际 OpenClipboard 或系统并发复制证据。
原 watcher 提取外、配对派发外、原解码/解析与九项正文、十六份关联文件核对；隐藏扩展
API 仅 Windows，原其它接口保持；未增加依赖/SDK feature。
干净源码 b541e87dd0125307abe38d50b33cf0940d596427 完整 Windows 默认/QA 门禁 child/终端 exit 0，30 passed /
0 failed / 1 Linux smoke skipped。默认 Rust 1183、QA Rust 1246（各 5 ignored，重叠不累加），
新八项/旧 23 项各图通过；独立 guard 七项在原 CF_HTML 入口，CF_HTML 共 16/剪贴板共
31 项，vendor WGC 十八项另列。前端 79 文件/1323 passed，Python 33 + 3、check/严格 lint/
供应链/构建入口通过。五份源码/日志/checked helper/干净检出核对，门禁后仅四份 Markdown。
累计二十八项本机产品修复未装包；保存实际 QA/模板保持。实际系统/延迟渲染提供者、
其它宿主/当前 SHA CI、安装/Wayland 回归留未验，桌面操控停止。
证据 windows-clipboard-snapshot-contract / windows-clipboard-snapshot-native-qa-b541e87。
