# Clippy 当前架构

## 技术边界

- 主窗口：vanilla HTML/CSS/ES modules，保留稳定的剪贴板高频交互。
- Pin、截图覆盖层、图片编辑器：React + TypeScript 功能岛。
- 系统资源：Rust/Tauri 拥有剪贴板、数据库、窗口、截图帧、Portal 会话、Pin 数据、密钥和网络请求。
- IPC：`src/js/api.ts` 是唯一 Tauri 调用边界，`ipc-types.ts` 对齐 Rust serde 字段。

## 后端模块

| 模块 | 职责 |
|---|---|
| `lib.rs` / `app/` | `lib.rs` 组装 Tauri builder、managed state 和 Wayland gsettings/D-Bus；`app/` 管理开发自启防护、WebKit 诊断、托盘、X11 快捷键和窗口事件；托盘菜单文案取自 `i18n.rs`，`config-changed` 同时刷新图标（主题）与菜单文案（语言） |
| `commands/` | 按 clipboard/settings/tmux/capture/OCR/URL 拆分薄 IPC 命令 |
| `clipboard_watcher.rs` + `clipboard_watcher/*` | 主轮询与去重协调；内容分类、写入重试和 tmux/inotify 监听各自隔离 |
| `paste/mod.rs` | 自动粘贴协调器、后端选择、Copy-only fallback 和稳定状态契约 |
| `paste/portal.rs` / `x11.rs` / `token_store.rs` | Portal 会话与授权状态机、X11 窗口恢复与输入、私有 restore token 持久化 |
| `window_controller.rs` | 主窗口 work area、logical/physical 尺寸与位置约束；设置窗口的开窗入口（托盘与 IPC 共用一份几何与标题） |
| `i18n.rs` | 托盘菜单与 Rust 侧窗口标题的静态文案；语言解析与前端 `i18n.js` 同规则（显式值优先，`auto` 看 `LC_ALL`/`LC_MESSAGES`/`LANG`，其余回退英文） |
| `capture/` | 单一 CaptureSession、冻结帧、多显示器覆盖层、裁剪与动作 |
| `screenshot.rs` + `screenshot/*` | 原始截图帧契约与 PNG 编解码；Wayland/Portal/GNOME/xcap fallback 和几何测试隔离 |
| `pin/` | PinManager、内容来源、窗口尺寸、缩放/透明度/锁定和清理 |
| `translation/` | provider、超时/重试、request-id、内容选择、Secret Service；启用的服务按 `spawn_blocking` 并行，单服务失败作为数据返回；`direction.rs` 在文本已是目标语言时按备选语言换向；`tts.rs` 走 dictvoice 取回音频 |
| `storage.rs` + `storage/*` | SQLite/FTS5 初始化与搜索；维护清理、统计、URL 缓存、翻译记录和测试各自隔离 |
| `image_io.rs` / `dialogs.rs` | PNG 与剪贴板互转；按配置的目录与文件名模板落盘（`SaveTarget`）；另存为与选目录对话框只在 `dialogs.rs` 调用插件 |

## 前端模块

| 模块 | 职责 |
|---|---|
| `js/clipboard-list.js` | 列表 facade、数据加载、IPC 动作与增量渲染装配 |
| `js/clipboard/` | 导航状态机、展示格式化和单行 DOM/缩略图渲染 |
| `js/preview-panel.js` | 预览状态、检测优先级、延迟库与缓存 |
| `js/preview/*-renderers.js` | 代码、元数据、格式、加密、内容/OCR 渲染 |
| `react/main/translationStore.ts` | 主预览翻译状态、多服务结果卡、单服务重试与陈旧响应保护 |
| `styles/components.css::.translation-host` | React 翻译面板的挂载壳：`.preview-panel` 的 flex 子项必须自己有 `min-height: 0` 与 `max-height`，否则 `.translation-panel` 的百分比 `max-height` 相对 auto 高度失效，翻译区会把预览内容挤没、自己被窗口裁掉。主窗口高度对所有面板组合恒定（`MAIN_WINDOW_HEIGHT`），翻译区靠这个上限和自身滚动落位——高度随预览变化会连带改变列表可见行数，列表跟着重排比翻译区挤一点更难用 |
| `js/translation-providers.ts` | 服务显示名、默认端点与能力标记（设置页/主面板/选区翻译共用） |
| `react/capture-overlay/` | 窗口命中、选区移动/缩放、提交动作（直开编辑器/停在工具条）、直接动作与选区翻译 |
| `react/capture/` | 16 个标注工具（选择/绘制/效果三组）、图像调整、撤销/重做和统一导出；视口、PNG 管线及待处理截图加载器独立管理 |
| `js/settings/` | 主题、自动粘贴授权、快捷键录制与注册失败提示、OCR 和统计控制器 |
| `react/pin/` | 首帧就绪、工具栏、拖动阈值和 rAF 更新合并 |

## 主窗口键盘状态机

键盘归属按**焦点位置**单点解析（`js/keyboard-router.js::resolveKeyboardMode`），先匹配先赢：
`codec > search > translation > list`。不按"面板是否可见"判定，因为键盘操作下焦点不会自己跑出侧栏，
"只有 `` ` `` 能切回列表"才成立；而鼠标点回中间列表能立刻把键盘交还列表。

| 模式 | 谁拥有键盘 | 被路由拦下的键 | 其余按键 |
|---|---|---|---|
| `codec` | 焦点在 `#codec-panel` 内（`` ` `` 打开左侧栏） | `` ` ``、`Esc` 关面板并把焦点还给列表 | 全部交给面板自己（字母/数字打进输入框，不驱动列表） |
| `codec` 内层 | 自定义下拉展开中 | `Esc` 只收起下拉并 `stopPropagation()`，不外泄给上表 | 收起后再按一次 `Esc` 才关侧栏 |
| `search` | 搜索框聚焦 | `Esc` 逐级退出（清空 → 收起 → 隐藏窗口） | 交给 input，`` ` `` 能正常打出反引号 |
| `translation` | 焦点在 `#translation-react-root` 内（`Shift+Tab` 显式送入） | `Ctrl+Enter` 翻译；`Esc`/`Tab` 只把焦点交回列表（预览留着）；`` ` `` 切侧栏 | 放行原生滚动与按钮语义 |
| `list` | 默认（预览开着也一样） | `w/a/s/d`、方向键、`1-9/0`、`Enter`、`Space`、`Ctrl+P`、`Ctrl+Enter`、`Tab`/`Esc` | 带修饰键的其它组合一律放行 |

方向键与 `ws` 在 `list` 模式下**始终**驱动列表：Tab 打开预览不再把焦点塞进翻译面板，
焦点要进翻译区必须显式 `Shift+Tab`。焦点撤离翻译面板时先 `blur()` 再 focus `#list-panel`，
不留"谁也不拥有"的中间态。

"内层先吃掉、吃掉就不外泄"是这套状态机的通用规则：嵌套控件消费了某个键就必须
`stopPropagation()`，否则一次 `Esc` 会同时收下拉 + 关侧栏，输入框里的内容跟着一起没了。
反过来，路由拦下一个键之前也要确认动作真的发生了（`Shift+Tab` 只在焦点确实进了翻译面板时才
`preventDefault`；翻译进行中按钮 disabled、无选中条目时面板 render 成 null，这时把键还给浏览器）。

codec 侧栏的操作可以显式收藏（`codec-favorites` 分组，localStorage `clippy-codec-favorites`），
星星按钮用 `js/icons.js` 的描边/实心两个图标切换状态，与列表行的收藏按钮同一套图标。
收藏是跨版本的外部输入：`init()` 会剔掉下拉里已不存在的操作并回写自愈，且不把值拼进 CSS
选择器（带引号的脏值会让 `querySelector` 抛错，`codec.init()` 中断则整个主窗口初始化不完）。
面板里的操作名、分组标题、按钮提示和输入框占位符全部挂 `data-i18n`（专有名词如 Base64/JWT/SHA 不翻译），
下拉触发按钮的文案与收藏分组是 JS 写入的，`applyToDOM` 碰不到，因此语言切换后由 `codec.refreshLabels()` 补齐。

失焦自动隐藏与这套状态机配套：`app/window_events.rs::should_hide_on_focus_loss(preview, codec)`
在预览或 codec 侧栏打开时豁免隐藏（纯函数 + 单测）。主窗口里也不允许出现原生 `<select>`——
WebKitGTK 的原生下拉是独立 GTK 弹窗，一打开 webview 就失焦，看着像窗口崩掉；
所有下拉都用 `js/custom-select.js`（支持分组标题与动态选项 `refresh()`），
`entrypoints-smoke.test.js` 锁定 `index.html` 里 `<select>` 数量为 0。

## 图片编辑器工具

分组即侧栏分组（`EditorSidebar.tsx::TOOL_GROUPS`，成员由 `capture-editor-tools.test.js` 锁定）。

| 分组 | 工具 | 说明 |
|---|---|---|
| 选择 | crop、object、eraser | 裁剪选区；选中/拖动已有标注；橡皮一次点击删一个标注（保持撤销粒度） |
| 绘制 | pen、marker、rect、ellipse、line、arrow、measure、text | 四种拖拽形态（折线/矩形/线段/文本）复用同一套包围盒、命中与移动逻辑；marker 半透明且笔宽更粗，ellipse 只在轮廓附近命中，measure 标注原图像素长度 |
| 效果 | highlight、blur、mosaic、spotlight、magnifier | blur/mosaic/spotlight/magnifier 需要读取或压暗底图，因此始终先于矢量标注绘制，magnifier 从原图重采样使预览与导出清晰度一致；highlight 只是半透明矢量色块，按用途归在这一组，绘制顺序仍跟随矢量标注 |

## 核心流程

```text
clipboard item -> preview -> translate/copy

shortcut -> frozen monitor frames -> selection commit
         -> commit_action=editor  -> editor -> copy/save/pin
         -> commit_action=toolbar -> copy/save/pin
                                  -> local OCR -> text translation

clip/image/capture -> PinManager -> hidden window -> first frame ready
                   -> scale/opacity/lock/copy/save/edit -> destroy cleanup
```

选区提交后的默认动作由 `capture_commit_action` 决定（`"editor"` 直开编辑器，`"toolbar"` 停在覆盖层工具条；
认不出的值按 `"editor"` 处理，因此这个字段不需要配置迁移）。后端归一化后随
`CaptureOverlayPayload.commitAction` 下发，前端不再认识配置里的怪值。`editor` 模式下松手时按住 `Alt`
可以临时留在工具条上，选区翻译因此不会在默认配置下失去入口。

编辑器窗口按截图尺寸开：`commands/capture_editor.rs::editor_window_size` 把物理像素按 `scale_factor`
折算成逻辑尺寸，加上侧栏与 chrome 的固定占位，再夹在 `min_inner_size(820,560)` 与工作区（留 48px 边）之间；
复用已有窗口时同样重设尺寸并 `center()`。画布侧 `captureViewport.ts` 的 `fitScale` 允许上采样到
`MAX_FIT_SCALE = 3`，小选区不再是大窗里的一小块。窗口速选依赖 `xcap::Window::all()`，
枚举失败或返回空时后端记 `log::info`、覆盖层显示 "Window picking unavailable in this session"，
让 Wayland 下的退化可见；已有选区后在选区外仍然给窗口悬停预览。

## 自动粘贴状态

```text
X11     : capture _NET_ACTIVE_WINDOW -> hide Clippy -> restore/confirm -> Ctrl+V
Wayland : select keyboard + persist_mode=2 -> rolling restore token -> reused session
Fallback: permission/backend/injection failure -> clipboard remains populated, no key injection
```

快捷键注册失败（Wayland 桌面不受管、X11 组合被占用）由后端记账，`get_shortcut_failures` 可随时读取，因此启动阶段早于设置页监听的失败也能显示；设置页对同一动作只保留最新一条，保存成功后重新拉取。注册、保存后更新和录制结束的恢复三条路径都记账，且都按动作归因：X11 逐个 `register`（不用全有或全无的 `register_multiple`），GNOME 恢复逐个写 binding 后只重启一次 gsd，因此一个键位被占用不会连坐另外两个。全部失败才把状态退回"已暂停"，部分成功保持"已恢复"，否则录制期的暂停会被跳过。Clippy 内部三个快捷键互相冲突由前端判定（它能读到未保存的录制值），桌面级冲突由 Rust 判定，X11 无法枚举时明确报告"无法检查"而不是"无冲突"。

GNOME 自定义快捷键条目路径按 command 认领而不是写死 `custom0/1/2`：这些编号先到先得，用户自己建的快捷键很可能已经占用，直接覆盖 name/command/binding 会静默销毁它。启动时读一次 `custom-keybindings`，认出带 Clippy D-Bus 方法的条目就原地复用，认不出来的再取未占用编号，结果在进程内缓存（`gsettings_shortcuts::plan_slots`）。

设置窗口关闭时，快捷键录制控制器先等待 `resume_shortcuts` 完成再关闭；Rust `AppState` 以原子标志和转换锁提供窗口销毁后的幂等恢复兜底。截图编辑器的待处理截图由最新请求代次门控，后端读取不消费缓存，窗口销毁或显式清理时统一释放。

`XDG_SESSION_TYPE` 优先于残留的 display 环境变量。Portal token 不进入普通配置；独立文件必须为 0600。首次 Portal 确认、撤权和桌面后端是否允许静默恢复仍属于真实桌面人工验收。
截图 Portal 的交互模式由截图用户动作显式开启；后台或未来自动任务应传入非交互模式，避免隐式弹出桌面授权。

## 安全规则

- 敏感条目在 Rust 内容选择阶段拒绝翻译；朗读条目文本走同一条内容选择路径，因此同样被拒绝。
- 朗读音频由 Rust 取回后以 data URL 播放，webview 不直接请求 dictvoice；文本长度上限 200 字符。
- 图片翻译只把本地 OCR 文本发送给 provider，不上传原图。
- API key 只进入系统 Secret Service，不提供明文 fallback。
- 成功的译文与原文落在同一个 SQLite 库（`translation_history`，全库上限 500 条）：条目删除、历史清空和上限清理都会一并删除它的译文，设置里另有"清空已保存的译文"入口。敏感条目从不进入翻译，因此也不会产生记录。
- 上面这条"删条目必然删译文"以及 `clips` 与 `clips_fts` 的一致性由事务保证：`insert_clip`、`delete_clip`、`clear_history`、`delete_entries` 都用 `unchecked_transaction()` 包住多条语句（`StorageEngine` 只持有 `&self`，并发已由外层 `Arc<Mutex<_>>` 串行化）。没有事务时中途失败会留下"搜得到但已不存在"的 FTS 幽灵行或删不掉的译文，而 `rebuild_fts_once` 只在 schema 版本变化时跑，索引不会自己长回来。
- 截图保存目录与文件名模板可配置（留空即内置默认 `~/Pictures/Clippy`）：模板只生成文件名，路径分隔符与前导点被清洗，写不到目录之外；同名时追加序号，不覆盖已有文件。另存为的路径由系统对话框返回。
- 用户文本使用 React 文本节点或 `textContent`；富文本仅使用严格 DOMPurify 配置。
- URL 元数据仅访问无凭据的 HTTP(S)，拒绝私有/保留 IP、私有 DNS 解析和重定向；请求有 5 秒超时与 1 MiB 上限。
- 翻译响应有超时与 1 MiB 上限；数学表达式不使用 `eval`/`Function`。
- 非 2xx 响应只在 4xx 时读取最多 4 KiB 正文用于错误归类（把"缺少/无效 key"从不透明的 `http_status` 里区分出来），5xx 正文一律不读，网关错误页不会被误判成凭据问题。

## 质量门禁

`./scripts/ci-local.sh` 依次执行 Rust fmt/check/clippy/test、锁文件安装、TypeScript、Vitest、DOM/Xvfb smoke、Canvas 导出像素 smoke、主窗口布局像素 smoke 和 Vite build。两个像素 smoke 都需要 firefox 加 ffmpeg 或 python3-pil 读取截图像素，缺少时整步跳过（不算通过）。布局 smoke 直接 `?raw` 引入产品 `index.html` 的结构（headless Firefox 的 `--screenshot` 不等待顶层 `await`，异步 fixture 只会拍到空白页），断言失败时把原因画进红色浮层。criterion 基准（`src-tauri/benches/`，通过 `bench_support.rs` 调生产代码）被 `--all-targets` 编译但不运行，数字与运行方式见 [bench-baseline.md](bench-baseline.md)。Linux 发布目标仅为 deb/AppImage；updater 签名由 release CI secret 生成。
