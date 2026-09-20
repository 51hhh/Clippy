# PX-ACT-01 类型化动作与启动器规格

## Goal

让截图、OCR、扫码、翻译、复制、保存和 Pin 通过同一组后端声明、权限与请求身份运行，后续启动器
只负责发现和组合已登记动作，不复制领域实现，也不获得任意 IPC、文件、网络或进程权限。

## Requirements

1. 动作 ID、输入类型、输出类型、权限、是否可取消和平台范围由后端静态注册表唯一声明。
2. 图片输入只能引用后端已经持有的图片与修订代次；动作参数不得接收文件路径、任意 URL、原始像素
   或可执行命令。
3. 调用权限来自 Tauri 注入的真实窗口 label。主窗口、未来的启动器以及 Capture、Viewer、Pin
   功能岛各自只有显式动作集合；Settings、Longshot 控制窗和 Recording 控制窗没有动作执行权限。
4. 每次运行绑定 `caller + request slot + generation`。同一槽的新请求会使旧请求过期；取消、完成和
   发布结果都必须携带后端签发的精确 handle。
5. 日志和 `Debug` 输出只记录动作 ID、请求槽、代次和输入类型，不记录文字正文、图片引用值、翻译
   结果、文件路径或凭据。
6. `translation.network` 是第一阶段唯一网络权限；第一阶段权限枚举不包含外部进程、Shell、脚本、
   动态模块或任意网络端点。
7. 实际执行必须调用现有截图、OCR、扫码、翻译、剪贴板、保存和 Pin 领域服务；注册表不复制业务
   算法，IPC 适配器仍须经过 `ipc_access` 窗口权限矩阵。

## Built-in actions

| ID | Input | Output | Permission | Cancellable |
|---|---|---|---|---|
| `capture.start` | `unit` | `capture_session` | `screen.capture` | 否 |
| `image.ocr` | `owned_image` | `recognized_text` | `image.local_analysis` | 是 |
| `image.scan_codes` | `owned_image` | `detected_codes` | `image.local_analysis` | 是 |
| `text.translate` | `translation_request` | `translated_text` | `translation.network` | 是 |
| `text.copy` | `text` | `unit` | `clipboard.write` | 否 |
| `image.save` | `owned_image` | `saved_path` | `file.write` | 否 |
| `image.pin` | `owned_image` | `window_handle` | `window.create` | 否 |

`owned_image` 只含受限 `sourceId` 与 `sourceVersion`。运行适配器必须再次确认该引用属于当前窗口或
当前动作组合，不能把仅通过 JSON 形状校验当成所有权证明。

## Lifecycle

```text
discover descriptor
  → validate caller role + action permission
  → validate bounded typed input
  → begin(caller, request slot)
  → execute existing domain service with cancellation token
  → check exact generation
  → publish typed result once
  → remove active slot
```

同一槽开始新动作时，旧 token 立即进入 cancelled；旧 worker 即使稍后成功，也会因 generation 不匹配
而得到 `superseded`，不能写入新图片、新选区或新翻译面板。显式取消保留当前槽直到 worker 观察并
完成清理，使调用方能够区分 `cancelled` 与 `superseded`。

## Acceptance Criteria

- 注册表的 ID 唯一、稳定且可序列化，未知动作和未知字段会被拒绝；
- 每个内置动作都有合法与非法参数 fixture，正文/路径不会出现在调试文本中；
- 子窗口权限矩阵覆盖允许与跨域拒绝；
- 同槽替换、显式取消、迟到完成、错误完成和正常完成均有并发状态测试；
- 业务适配器接入后，发现、运行、取消和错误通过受限 IPC 端到端验证；
- 启动器 UI 接入后，键盘操作、空状态、运行中、取消和错误状态通过 DOM/真实窗口验收。

## Out of Scope

- Lua、WASM、JavaScript 或其它脚本运行时；
- 任意 Shell、外部进程、动态库和插件市场；
- 用户自定义网络端点或把敏感正文写入动作历史；
- 用动作注册表绕过 Viewer、Capture、Pin 已有的会话 handle 与所有权检查。

## Delivery stages

1. **Core contract**：静态注册表、参数校验、调用角色、取消与 generation 运行时；无 IPC、无 UI。
2. **Domain adapters**：逐个复用既有服务，并为每个动作加入所有权复核与可取消边界。
3. **Restricted IPC**：新增独立 Launcher 窗口角色；发现/运行/取消命令进入访问矩阵。
4. **Launcher UI**：键盘优先的动作搜索与参数表单；不显示或执行未授权动作。
5. **Composition**：只有类型可连接的输出才能进入下一动作，整条链保留同一根请求身份。

## Implementation status

- 2026-09-20：Stage 1 已完成后端注册表、内部类型转换、角色权限、取消与 generation 闸门。
- 2026-09-20：Stage 2 已接入 `text.copy`，复用产品唯一剪贴板写入路径；不可取消副作用在提交期间
  锁定同一请求槽。其余领域适配器、IPC、UI 与组合仍未完成。
