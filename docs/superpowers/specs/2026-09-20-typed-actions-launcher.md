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

### Stage 5 composition contract

- 主窗口可以把当前历史图片的数字 clip ID 作为 Launcher 上下文；托盘入口使用最新历史图片。后端在
  64 MiB PNG、16,384 px 单边和 32 Mpx 预算内读取并校验图片，冻结为当前 Launcher 窗口独占的
  不可变快照，再签发 `sourceId + sourceVersion`。前端不能提交路径、URL 或图片字节，也不能把
  普通 JSON 形状当作图片所有权。
- 直接图片动作只接受这份签发快照。窗口销毁会同时回收快照、尚未提交的动作和已完成组合节点；
  删除或更新原历史条目不改变已经冻结的本次动作输入。
- 完成结果以原动作 handle 作为后端组合引用。下一动作只提交上游 handle、目标动作、请求槽和不含
  正文的目标选项；OCR 正文和译文始终从后端保存的精确结果派生，不能由前端回传替换。
- 第一阶段只开放 `recognized_text → text.copy`、`recognized_text → text.translate` 和
  `translated_text → text.copy`。扫码可能包含多个结果，不隐式拼接；截图 session、保存路径和窗口
  handle 也不作为后继输入。
- 每个完成节点记录原根身份、调用窗口、输出类型和创建代次。跨窗口 handle、未知/已回收 handle、
  类型不兼容、未知选项、超长派生正文和同槽提交竞态均返回稳定错误码；组合后的输出继续继承同一
  根身份并可按上述映射连接。

## Implementation status

- 2026-09-20：Stage 1 已完成后端注册表、内部类型转换、角色权限、取消与 generation 闸门。
- 2026-09-20：Stage 2 已接入 `text.copy`，复用产品唯一剪贴板写入路径；不可取消副作用在提交期间
  锁定同一请求槽。其余领域适配器、IPC、UI 与组合仍未完成。
- 2026-09-21：Stage 2 已接入 Viewer 的 `image.ocr`。动作只能解析调用窗口后端签发的不可变
  `snapshotId + version 0`，复用现有 OCR single-flight、并发预算和进程回收；动作取消会释放自身
  等待者，迟到、跨窗口、旧版本和已关闭 Viewer 均不能发布结果。Capture、Pin、主窗口和 Launcher
  的图片引用仍须先建立各自权威 source/version 合同。
- 2026-09-21：Stage 2 已接入 Viewer 的 `image.scan_codes`。它复用与 OCR 相同的不可变快照所有权
  合同，以及现有 QR/条码扫描的进程级单并发预算；Viewer、历史图片命令和动作适配器共享同一
  blocking 执行入口，扫描大图不再为 Viewer 额外复制完整 PNG。取消或同槽替换会立即释放动作
  等待者，不可中断的 rxing worker 仍持有 permit 到安全结束，迟到结果不能发布。
- 2026-09-21：Stage 2 已接入 `text.translate`。它复用当前配置、方向解析、系统 keyring、provider
  路由和响应限制，但为每次动作建立独立的领域 request-id 空间，避免主窗口、Viewer、Launcher
  或不同动作槽通过全局 `latest_request_id` 互相淘汰。取消和同槽替换会立即释放动作等待者；
  已进入同步 provider 的请求仍按既有超时结束，动作 generation 闸门拒绝其迟到结果。错误仅保留
  稳定动作码，不记录正文、译文、provider 响应或凭据。当前直接文本动作只授权 Main/Launcher；
  Viewer/Capture 继续使用会复核敏感状态的专用翻译命令，待组合阶段能携带可信文本来源后再开放。
- 2026-09-21：Stage 2 已接入 Viewer 的 `image.save`。动作保存调用窗口后端签发的精确不可变扁平
  快照；可编辑工程、未提交画布文档和保存模式仍由 Viewer 专用命令处理。所有权复核与原子文件
  写入位于同一个不可取消提交阶段，同槽请求在落盘期间不能替换它；成功结果返回实际路径，错误和
  日志不包含图片引用、用户目录或底层文件系统消息。
- 2026-09-21：Stage 2 已接入 Viewer 的 `image.pin`。动作仅把调用窗口持有的精确不可变扁平快照
  交给现有 Pin 建窗服务；可编辑工程和未提交画布继续走 Viewer 专用命令。所有权复核、会话级串行
  与建窗处于同一不可取消提交阶段；原生 builder 调用后的失败会返回独立的“不确定”动作码，并在
  Viewer 会话上保持粘性状态，后续动作和专用命令都不能自动重试而产生重复窗口。
- 2026-09-21：Stage 2 已接入 `capture.start`。动作复用托盘、快捷键和主窗口的唯一普通截图入口，
  返回后端签发的真实 session ID，并保留模式 gate、多屏冻结、源窗口隐藏恢复和覆盖层创建补偿。
  异步启动在独立不可取消提交任务中完成；即使动作等待者随窗口关闭被丢弃，领域 future 仍会走到
  成功或补偿终点并回收精确动作槽。平台错误只映射为稳定动作码，不暴露窗口或后端细节。
- 2026-09-21：Stage 3 已完成受限 IPC。`discover_actions`、`prepare_action`、`run_action` 和
  `cancel_action` 全部经过统一 `ipc_access` 矩阵；调用角色只取 Tauri 注入的原生窗口 label，Settings、
  Longshot 与 Recording 控制窗不能调用。`prepare` 是 IPC 唯一接收输入的位置，Rust 在校验并转换后
  把类型化输入留在动作槽，`run` 只接受后端签发的 `requestSlot + generation` 句柄；句柄只能领取
  一次，未知字段、跨窗口句柄、重复运行和过期结果均被拒绝。可取消等待者被丢弃时会取消并回收仍在
  pending 的槽；每窗口最多 16 槽、全局最多 64 槽，窗口销毁会回收 pending 输入，已进入截图等
  不可取消提交的任务继续完成领域补偿。前端受控 facade 再次严格校验
  静态目录、句柄身份、输出联合和 OCR/扫码/翻译嵌套结果。错误响应只含稳定码。Main/Launcher 的
  `owned_image` 在建立其权威图片来源前会稳定返回 source unavailable。
- 2026-09-21：Stage 4 已实现独立无边框 Launcher。托盘和主窗口 `Ctrl/Cmd+K` 共用唯一建窗入口，
  页面首帧与原生关闭监听就绪后才显示；搜索、方向键、Enter、Escape 与 `Ctrl/Cmd+Enter` 覆盖完整
  键盘路径。UI 只从受限目录中展示自己能构造直接输入的 `capture.start`、`text.copy` 和
  `text.translate`；四个 `owned_image` 动作在 Stage 5 获得可信来源前保持隐藏。运行界面按 descriptor
  决定是否允许取消，关闭可取消任务时先取消精确 handle；错误只映射稳定码，不显示后端细节。
  Launcher 启动截图时会随主窗口一起从冻结帧隐藏，成功建出覆盖层后销毁自身。DOM 回归覆盖目录
  筛选、键盘选择、空状态、运行、取消、原生关闭和错误。隔离 X11/D-Bus 原生 smoke 已确认真实
  Tauri 窗口在首帧后以 640×520 创建；当前宿主的 Wayland 合成器交互和 Windows/macOS 仍由各自
  原生 CI/QA 分层验证，不能由该 smoke 代替。
- 2026-09-21：Stage 5 已接入可信图片与受限组合。主窗口传递当前图片 clip ID，托盘选择最新图片；
  Rust 在字节、尺寸和像素预算内读取、完整解码并冻结 Launcher 独占 PNG，只把不透明来源引用与
  展示尺寸交给 WebView。OCR、扫码、保存和 Pin 复用既有领域 adapter；完成的 OCR/译文由后端按
  精确 handle 短期保存，下一动作只提交上游 handle 和语言选项，正文不会回传。当前只允许
  OCR→复制/翻译和译文→复制，跨窗口、过期、类型不兼容和窗口关闭后的迟到结果均被拒绝。整条链
  继承同一根安全身份：敏感图片可本地识别和复制但不能联网翻译；即使来源加载后才被标记敏感，
  provider 调用前的内容哈希复核也会阻断。Launcher 窗口销毁会回收图片、完成节点和 pending 动作。
