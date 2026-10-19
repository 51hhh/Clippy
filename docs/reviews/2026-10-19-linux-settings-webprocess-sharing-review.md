# Linux 设置页 WebProcess 复用验证

Requirement ID：`LINUX-SETTINGS-WEBPROCESS-01`

## 结论

Linux 设置页关联主窗口的 WebView 后，真实设置窗口仍是 720×560，加载、主题切换和受保护关闭均
可用；打开设置页不再创建第二个 `WebKitWebProcess`。同机、同一 Vite 前端与隔离 X11/D-Bus
流程下，设置页打开增量 PSS 从 85,950 KiB 降至 17,401 KiB，单次样本降低 68,549 KiB
（79.8%）。该数值是相对采样，不代表其它桌面、WebKit 版本或工作负载的绝对收益。

设置页关闭 15 秒后，关联 WebProcess 仍比首次打开前高 6,645 KiB PSS；切换主题的另一轮运行中
关闭 42 秒后仍高 19,386 KiB。共享进程会保留一部分 WebKit/页面热缓存，不能宣称关闭后完全回到
冷启动占用；本改动优化的是设置页打开期间的进程固定成本与峰值，并以较小的热缓存留存作为权衡。

## 实现边界

- Linux 主窗口存在时，在 `WebviewWindow::with_webview` 主线程回调内把平台 WebView 交给
  `WebviewWindowBuilder::with_related_view`。GTK WebView 句柄不跨线程传递。
- 调用方只通过有界 channel 等待 `Result<(), String>`；同步与异步派发均可工作。
- 派发失败、建窗失败和两秒超时都明确返回错误。超时后不回退创建独立窗口，避免迟到回调生成重窗。
- 主窗口缺失时继续走独立 builder；非 Linux 平台完全保留原路径。
- 共享意味着 Linux 主窗口与设置页共用 WebProcess 崩溃边界；两者仍是独立窗口、WebView 和 DOM。

## 自动化合同

`window_controller::tests` 新增七项：

1. 主窗口缺失时调用独立 builder；
2. 主窗口存在时优先关联 builder；
3. 同步派发成功；
4. 异步派发成功；
5. 派发失败保留上下文；
6. builder 错误原样返回；
7. 等待超时且不触发回退。

定向执行结果：7 passed / 0 failed。严格 `cargo clippy --all-targets -- -D warnings` 通过。

## 完整本地门禁

非沙箱执行 `./scripts/ci-local.sh`：25 passed / 0 failed / 2 skipped。核心 Rust 测试
1326 passed / 14 ignored，前端 Vitest 81 files passed / 2 skipped、1363 tests passed /
36 skipped；X11 录屏闭环、X11 剪贴板隔离、DOM/Xvfb、Canvas 像素、主窗口布局和 Vite build
均通过。跳过项是非宿主平台交叉 lint 与需显式启用的 AppImage 可视 smoke，不计为通过。

受限沙箱内的首次门禁因 loopback bind、X11 socket 与子进程 `EPERM` 得到 19/6/2，该结果是环境
阻断而非通过记录；移出相应沙箱后同一工作树完整重跑得到上述 25/0/2。Windows/macOS 条件编译
不在本机编译图内，同 SHA Native Check 仍未执行。

## 实机流程与采样

两组实现均使用直接 `cargo build --release --bin clippy-app` 产物，并连接同一
`http://127.0.0.1:1420` Vite 前端。每次运行使用全新的 XDG config/data/cache/runtime 目录、
独立 session D-Bus、Xvfb 1280×800×24、`GDK_BACKEND=x11` 与
`WEBKIT_DISABLE_COMPOSITING_MODE=1`。手动映射隔离会话中的 16×16 托盘窗口，从真实托盘菜单
选择 Settings；进程值读取 `/proc/<pid>/smaps_rollup`，Private 为 `Private_Clean +
Private_Dirty`。短暂的 glycin/bwrap 图片解码进程不计入两边总数。

### 修改前：独立 WebProcess

| 状态 | Core PSS | Network PSS | 主 Web PSS | 设置 Web PSS | 进程组 PSS | Private |
|---|---:|---:|---:|---:|---:|---:|
| 设置关闭 | 72,981 | 14,126 | 87,680 | — | 174,787 | 124,980 |
| 设置打开 | 75,476 | 14,252 | 80,542 | 90,467 | 260,737 | 188,860 |
| 打开增量 | | | | | **85,950** | **63,880** |

### 修改后：关联主 WebProcess

| 状态 | Core PSS | Network PSS | 共享 Web PSS | WebProcess 数 | 进程组 PSS | Private |
|---|---:|---:|---:|---:|---:|---:|
| 设置关闭 | 47,678 | 14,191 | 88,805 | 1 | 150,674 | 107,844 |
| 设置打开 | 51,752 | 15,049 | 101,274 | 1 | 168,075 | 122,520 |
| 打开增量 | | | | **0** | **17,401** | **14,676** |
| 关闭 15 秒 | 50,550 | 14,271 | 92,498 | 1 | 157,319 | 110,816 |

PSS 会随系统中共享库的其它使用者变化，因此不比较两次运行的冷态绝对总量，只比较各自运行内的
打开增量。RSS/VmSize 未用于收益结论。每个状态只有一次稳定快照，不构成统计基准测试。

## 桌面 QA

- `xwininfo`：`Clippy Settings` 为 720×560、可见、无第二个系统标题栏。
- 真实 WebView：General 内容与自绘标题栏完整渲染；点击 Dark 卡片后主题即时切换。
- 进程树：打开前、打开中、关闭后均只有 PID 不变的一个 `WebKitWebProcess`。
- 点击自绘关闭按钮后设置窗口从 X11 树消失，主 WebProcess 与 Clippy Core 继续存活。

Xvfb 没有窗口管理器，不能验证拖动、最大化、最小化、工作区定位或真实 GPU 合成。Wayland、
Windows、macOS、安装包以及同 SHA 原生 CI 未验证，不计为通过。
