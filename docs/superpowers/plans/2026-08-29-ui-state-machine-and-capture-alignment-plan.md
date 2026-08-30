# 主窗口状态机修复 + 截图链路对齐方案

制定日期：2026-08-29
适用分支：`dev`（本地领先 `origin/dev` 3 个提交）
状态：**已实施（2026-08-30）**，修 1～修 5 全部落地，`./scripts/ci-local.sh` = 11 通过 / 0 失败 / 1 跳过。
"不做"清单（滚动截图、Board、原地 Konva 标注、Pin 落回选区原位）保持不做。
UI 细节与真机行为仍需 `cargo tauri dev` / 真实桌面人工确认，条目见
`.trellis/tasks/08-08-clippy-integrated-refactor/qa-matrix.md` 的"真实桌面人工矩阵"。

## 0. 前置结论：自动粘贴授权已是长期授权

- Wayland 走 RemoteDesktop Portal，`PersistMode::ExplicitlyRevoked`（= persist_mode 2）：
  `src-tauri/src/paste/portal.rs:174-177`。授权一次后，只有用户在系统设置里显式撤权才会失效。
- restore token 存在独立 0600 文件里滚动更新（`paste/token_store.rs:21-33`，
  `portal.rs:202-247`）：成功就写新 token，失败按阶段决定保留还是删除
  （`PortalAuthorizationStage`，`portal.rs:70-107`），所以重启进程也不用重新授权。
- 同一进程内会话直接复用，不重新请求。
- X11 路径（`paste/x11.rs`）不需要任何授权。
- **例外提醒**：截图在 Wayland 上走的是 Screenshot Portal / GNOME Shell D-Bus
  （`screenshot/backends.rs:202-245`），那条路径**没有 restore token**，与粘贴授权不共享；
  真机上如果每次截图都弹授权，是这条链路的限制，不是粘贴授权退化。

---

## 1. 五个缺陷的根因（已定位到行）

### 缺陷 1：Tab 侧栏下翻译区被遮挡

`index.html:78` 的 `<div id="translation-react-root">` **在 CSS 里完全没有样式**，React 把
`.translation-panel` 渲染在它内部（`react/main/TranslationPanel.tsx:113`）。于是：

- `.translation-panel { max-height: 48% }`（`styles/components.css:121-127`）的百分比是相对
  `#translation-react-root` 的高度算的，而这个 div 高度是 auto（不确定），按 CSS 规则百分比
  max-height 直接失效 → 翻译区高度不受限。
- `#translation-react-root` 是 `.preview-panel` 的 flex 子项，默认 `flex: 0 1 auto` 且
  `overflow: visible` → `min-height: auto` = 内容高度 → **不能被压缩**。
- `.preview-content { flex: 1 }`（basis 0）在没有余量时被压到 0。
- 主窗口高度恒定 500px（`window_controller.rs:9`），`.preview-panel { overflow: hidden }`
  把溢出部分直接裁掉。

结果就是：翻译区把预览内容挤没了，自己的下半部分（结果卡、按钮）又被裁在窗口外 = "被遮挡"。

### 缺陷 2：Tab 打开后上下键/ws 翻动翻译组件而不是列表

`keyboard-router.js:141-150`：Tab 打开预览后主动调 `focusTranslationPanel()`，把焦点塞进
`#translation-react-root`。紧接着 `keyboard-router.js:39-45` 的第一条守卫
`if (inTranslationPanel && key !== "Escape" && key !== "Tab") return;` 把
`ArrowUp/ArrowDown/w/s` 全部放行给 DOM，`clipboardList.moveRow()` 根本没被调用；
而 `.translation-panel { overflow-y: auto }` 是焦点元素最近的可滚动祖先，于是按键变成滚动翻译区。

### 缺陷 3：` 打开左侧栏后拿不到键盘优先级

`keyboard-router.js` 里**只有 `case "\`": codec.toggle()` 这一处和 codec 有关**，
没有任何"codec 已打开"的分支。`codec.toggle()` 确实会 focus `#codec-input`
（`codec.js:79-90`），但路由的 `switch` 对 `w/a/s/d/1-9/Enter/Space` 一律
`preventDefault()` 并驱动列表（`keyboard-router.js:72-121`）——既误操作列表，又让这些字符
**打不进 codec 的输入框**。

### 缺陷 4：编码下拉框点击导致窗口隐藏/看着像崩溃

- `index.html:16-49` 用的是**原生 `<select>`**。WebKitGTK 的原生下拉是独立 GTK 弹窗，
  一打开 webview 就失焦。
- `app/window_events.rs:69-88` 的 `hide_main_after_focus_loss`：延迟 200ms 后，
  **只有 `preview_visible` 为真才豁免**，`codec_visible` 不在判断里 → 主窗口被 `hide()`。
- 主窗口是 `alwaysOnTop + decorations:false + skipTaskbar`（`tauri.conf.json`），父窗口一藏，
  那个 GTK 弹窗就成了孤儿浮层，视觉上和崩溃没区别。
- 前端 `app.js:120-125` 的 `onWindowBlur` 已经正确豁免了 codec，只有 Rust 侧漏了。

### 缺陷 5：截图链路与 flashot 的差异（调研结论要先纠正一个前提）

调研了 `example/flashot` 源码，**flashot 截完图并不会开一个新窗口**：

- 选区 mouseup → `commitDrag()`（`example/flashot/src/overlay/state.ts:290-307`）在**同一个
  全屏覆盖层里**原地进入标注态（`src/routes/Overlay.tsx:663-697`），懒加载 Konva 画布。
- 覆盖层 = 每显示器一个、和显示器等大、`objectFit: fill`，**图像 1:1，没有缩放也没有 zoom**
  （`src/overlay/FrozenLayer.tsx:97-107`，`src/annotation/Stage.tsx:1235-1243/1671-1680`）。
- 工具是两条**浮动可拖动条**，不是侧栏：横向标注条（13 个工具 + undo/redo，
  `src/annotation/Toolbar.tsx:205-271`）+ 纵向动作条（40×308，pin/取色/调整/滚动截图/
  关闭/保存/复制，`src/overlay/Toolbar.tsx:171-290`），位置由
  `src/lib/geometry.ts:5-108` 决定（优先选区下方 / 选区右侧，越界翻转）。
- 唯一的"新窗口"是 **Pin**，且只在用户点 pin 时出现：窗口尺寸 = 选区 + padding，位置
  = 显示器原点 + 选区坐标 − 24，**落在选区原地而不是居中**；只有 Pin 才有 50%–300% 缩放
  （`src-tauri/src/commands.rs:846-1031`，`src/routes/Pin.tsx:37-49`）。
- 窗口枚举：X11 用 `xcap::Window::all()` + XCB 读 `_NET_FRAME_EXTENTS`/`_NET_ACTIVE_WINDOW`
  （`src-tauri/src/window_probe/linux.rs:9-72`）；**Wayland 拿不到窗口列表**，退化为整屏。
  悬停高亮只有描边 + 微光，**不显示窗口标题**（`src/overlay/DetectHighlight.tsx:6-28`）。

对照 Clippy 现状：

- 窗口枚举、悬停高亮、点击选窗**已经实现**：`capture/window_probe.rs`（同样是
  `xcap::Window::all()`，按显示器裁剪、过滤自身 pid 和 <20px 的窗口）、
  `react/capture-overlay/useSelection.ts:58-70`（<4px 位移视为点击 → 选中悬停窗口）、
  `overlay.css:8-10` 的 `.window-preview` 描边。所以"应该可以获取桌面窗口大小快速选区"
  这一条 Clippy 已经有了，Wayland 下枚举为空则退化，行为和 flashot 一致。
- 真正的差异是两点：
  1. **提交选区后不进标注态**，要在浮动工具条上点 "Edit"（`OverlayToolbar.tsx:31-36`）
     才 `queue_capture_for_editor` 开 1180×760 的 `capture` 窗口
     （`commands/capture_editor.rs:39-84`）。用户看到的"截完图什么都没有"就是这里。
  2. **编辑器窗口尺寸与图像无关**：固定 `inner_size(1180,760)` + `center()`，而
     `captureViewport.ts:33` 的 `fitScale = min(maxW/nw, maxH/nh, 1)` **上限被钳到 1**
     → 小选区在大窗口里显示成一小块，四周全是空白，视觉上就是"图片没有正确缩放"。
     HiDPI 下 PNG 是物理像素、舞台是逻辑像素，这个 1 的钳制还会让 2x 屏的截图看起来偏大一格。

---

## 2. 状态机设计（这是修 1–4 的共同底座）

### 2.1 唯一真值来源：按焦点位置解析模式

在 `keyboard-router.js` 里新增一个纯函数（可单测），每次 keydown 解析一次，**先匹配先赢**：

| 优先级 | 模式 | 判定条件 | 键盘归属 |
|---|---|---|---|
| 1 | `codec` | `e.target` 在 `#codec-panel` 内 | 左侧栏（textarea / 下拉 / 按钮）完全拥有键盘 |
| 2 | `search` | `activeElement` 带 `.search-bar-input` | 搜索输入框（现状逻辑不变） |
| 3 | `translation` | `e.target` 在 `#translation-react-root` 内 | 翻译面板（只在用户显式进入时才可能命中） |
| 4 | `list` | 其余（含预览打开但焦点在列表） | 中间列表 |

**为什么按焦点而不是按"面板是否可见"**：你要求"左侧栏打开后 wsad 不能触发主栏"，用可见性判定
也能满足；但按焦点判定额外送一件事——鼠标点回中间列表就自动把键盘交还列表，不必先关侧栏。
两者都满足"只有 ` 能切回"，因为键盘操作下焦点不会自己跑出侧栏。**推荐按焦点**。

配套保证：
- `codec.toggle()` 打开时已经 focus 输入框（`codec.js:85`），保证进入 `codec` 模式。
- 新增 `focusList()`：给 `#list-panel` 加 `tabindex="-1"`，关闭 codec/预览/退出翻译面板后
  显式把焦点收回列表，避免落在 `document.body` 这种"谁也不拥有"的中间态。

### 2.2 各模式的按键契约

| 键 | `codec` | `search` | `translation` | `list` |
|---|---|---|---|---|
| `` ` `` | **拦截**：关侧栏 + `focusList()` | 不拦截（当普通字符打进输入框） | 拦截：开侧栏 | 拦截：开/关侧栏 |
| `Escape` | **拦截**：关侧栏 + `focusList()` | 现状（收起搜索 → 收起展开 → 隐藏窗口） | 拦截：退出焦点回列表（不关预览） | 现状退栈：展开 → 预览 → 隐藏窗口 |
| `Tab` | 不拦截（原生在面板内换焦点） | 不拦截 | 拦截：回列表 | 拦截：开/关预览，**焦点留在列表** |
| `Shift+Tab` | 不拦截 | 不拦截 | 拦截：回列表 | 预览打开时拦截：把焦点送进翻译面板 |
| `w/s/↑/↓`、`a/d/←/→`、`1-9/0`、`Enter/Space` | **全部不拦截** | 不拦截 | 不拦截（原生滚动/按钮） | 现状 |
| `Ctrl+P` | 不拦截 | 不拦截 | 不拦截 | 现状（Pin） |
| `Ctrl+Enter` | 不拦截 | 不拦截 | 拦截：翻译当前条目 | 预览打开时拦截：翻译当前条目 |

要点：
- 缺陷 2 的修法就是表里 `list` 行的 `Tab`——**Tab 只切预览，不再调 `focusTranslationPanel()`**，
  焦点始终在中间列表，`w/s/↑/↓` 照旧走 `moveRow`。想动翻译面板的按钮，用鼠标或 `Shift+Tab`
  显式进入；补一个 `Ctrl+Enter` 直接触发翻译，避免"必须用鼠标点 Translate"。
- 缺陷 3 的修法就是 `codec` 列——除 `` ` ``/`Escape` 外一律不拦截。
- `codec` 模式下 `Escape` 也关侧栏（你只要求 `` ` ``）。理由：Escape 在本项目一直是统一退栈键，
  若在 codec 里按 Escape 直接隐藏整个窗口，输入中的内容会消失，比"多一个关闭键"更糟。

### 2.3 失焦不隐藏窗口的状态

`app/window_events.rs::hide_main_after_focus_loss` 改成：`preview_visible || codec_visible`
任一为真就豁免（现在只看 `preview_visible`，`codec_visible` 状态其实已经存在于
`AppState`/`MainWindowLayout` 里，只是没参与这个判断）。

| 状态 | 鼠标点到别处 | 说明 |
|---|---|---|
| 仅列表（无侧栏） | **隐藏** | 保持"零干扰"语义 |
| 右侧预览打开 | 不隐藏 | 现状 |
| 左侧 codec 打开 | **不隐藏**（本次修） | 与前端 `onWindowBlur` 对齐 |
| 两个都打开 | 不隐藏 | |

前端 `app.js:120-125` 已经是这个语义，改完两侧一致，**这一条同时消灭"点任何原生弹窗都可能
丢窗口"这一类问题**（不只是下拉框：将来的右键菜单、颜色选择、文件对话框同理）。

---

## 3. 逐项改动清单

### 修 1（翻译区被遮挡）

1. `src/index.html:78`：给容器加类 → `<div id="translation-react-root" class="translation-host">`。
2. `src/styles/components.css`：新增并调整
   ```css
   .translation-host { display: flex; flex-direction: column; flex: 0 1 auto;
                       min-height: 0; max-height: 55%; overflow: hidden; }
   .translation-panel { flex: 1 1 auto; min-height: 0; max-height: none; overflow-y: auto; }
   .preview-content   { min-height: 96px; }   /* 不再被翻译区压到 0 */
   ```
   （`max-height: 55%` 落在 `.preview-panel` 上，它的高度是确定的 100%，百分比才生效。）
3. ~~`src-tauri/src/window_controller.rs`：`MainWindowLayout::logical_size()` 增加高度维度——
   预览打开时 500 → 620~~ **已撤销（2026-08-30）**：加高会让列表可见行数随预览开关变化
   （6 行 ↔ 8 行），列表跟着重排比"翻译区挤一点"更难用。高度对所有面板组合恒定 500，
   翻译区靠 `.translation-host` 的 `max-height` 与自身滚动落位；布局像素 smoke 改用 780×500
   校验这个几何，单测 `main_window_layout_uses_base_and_visible_panel_widths` 锁定高度不变。
4. 自动化验证：jsdom 没有布局引擎，测不出遮挡。新增 `scripts/smoke-layout.sh`，复用
   `smoke-canvas-export.sh` 那套 headless Firefox + 读像素的手法：加载一个把主窗口结构
   固定成 780×500 的 fixture，用 JS 断言
   `translationPanel.getBoundingClientRect().bottom <= previewPanel.bottom` 且
   `previewContent.height >= 96`，通过就画绿块、失败画红块，脚本读像素判定。
   并入 `scripts/ci-local.sh`。

### 修 2 + 修 3（键盘状态机）

1. `src/js/keyboard-router.js`：
   - 新增导出 `resolveKeyboardMode(event, { codecEl, translationEl, searchFocused })`（纯函数，
     直接单测）。
   - `onKeyDown` 改成 `switch (mode)` 分派，按 2.2 的表实现；删掉现在那条把翻译面板一刀切
     放行的守卫（`keyboard-router.js:43-45`），改为 `translation` 模式下只拦
     `Escape/Tab/Shift+Tab/Ctrl+Enter`。
   - `Tab` 分支删掉 `focusTranslationPanel()` 调用；新增 `focusList()`；
     新增 `Shift+Tab` 进入翻译面板、`Ctrl+Enter` 触发翻译。
   - 工厂参数新增 `translation: { focus, blurToList, translate }` 适配器（保持可注入、可测，
     不在 router 里直接 import React store）。
2. `src/js/app.js`：注入 `translation` 适配器（`translationStore.translate()` 已存在，
   `react/main/translationStore.ts`），给 `#list-panel` 加 `tabindex="-1"` 的 focus 调用点。
3. `src/index.html:66`：`<div id="list-panel" class="list-panel" tabindex="-1">`；
   `styles/components.css` 里 `.list-panel:focus { outline: none }`。
4. 测试（`src/tests/keyboard-router.test.js`，现有 fake 结构可直接扩展）：
   - codec 打开且焦点在面板内：`w/s/a/d/1/Enter/Space` 都不 `preventDefault`、
     `moveRow/selectByIndex/activateFocus` 一次都不被调用；
   - 同上：`` ` `` 与 `Escape` 关侧栏并调用 `focusList`；
   - 预览打开且焦点在列表：`ArrowDown/s` 仍调 `moveRow(1)`；`Tab` 不再 focus 翻译面板；
   - 焦点在翻译面板：`ArrowDown` 放行（不 `preventDefault`），`Escape`/`Tab` 回列表，
     `Ctrl+Enter` 调 `translate`；
   - 搜索输入框聚焦：`` ` `` 不被拦截（能正常打出反引号）。

### 修 4（下拉框 → 失焦 → 窗口消失）

两处都要改，缺一不可：

1. **Rust 侧兜底**（治一类问题）：`src-tauri/src/app/window_events.rs:69-88` 增加
   `codec_visible` 豁免。把判断抽成纯函数 `fn should_hide_on_focus_loss(preview: bool,
   codec: bool) -> bool` 并加单测（现有 `hides_instead_of_closing` 旁边）。
2. **前端换掉原生 select**（治根，原生弹窗本身就不该出现在无边框浮动窗口里）：
   `index.html` 的 `<select id="codec-select">` 改成 `custom-select.js` 那套 DOM
   （`.custom-select-trigger/.custom-select-dropdown/.custom-select-option`，设置页已在用）。
   需要给 `src/js/custom-select.js` 补两个能力，并在 `src/tests/custom-select.test.js` 补测：
   - **分组**：现在只支持平铺 option，codec 有 5 个 optgroup（编码/哈希/格式化/转换 + 最近使用）
     → 支持 `.custom-select-group` 标题节点，且分组标题不可选中；
   - **动态刷新**：`optionEls` 现在是 init 时的快照（`custom-select.js:14`），而"最近使用"
     每次执行都会重建（`codec.js::_renderRecent`）→ 增加 `refresh()`/`setOptions()`，
     否则最近项点了没反应。
   `codec.js` 里 `_selectEl.value` / `addEventListener("change")` 两处改为控制器的
   `value` / `onChange`；`codec.test.js` 用的 DOM fixture 同步更新。
3. 顺带：`.codec-panel` 里 `<pre id="codec-output" tabindex="-1">` 保持可聚焦，
   点它时 `e.target` 仍在 `#codec-panel` 内，状态机判定不受影响。

### 修 5（截图链路对齐）

先说取舍：**不建议照搬 flashot 的"覆盖层内原地标注"**。Clippy 的编辑器已经是 16 工具 +
分组侧栏 + 撤销/重做 + 统一导出管线 + 一整套单测和像素 smoke（`react/capture/`，
`docs/architecture.md#图片编辑器工具`），搬到覆盖层等于把这套推倒重写成 Konva 风格的浮动条，
收益只是"长得像参考项目"。而你真正描述的诉求——"截完图显示一个窗口，有边栏工具"——
恰好就是 Clippy 现有的编辑器窗口，缺的是"自动出现"和"图像正确缩放"。

推荐做法（按收益/风险排序）：

1. **选区提交后自动开编辑器**（对齐 flashot "commit 即进标注态"的体感）：
   - `AppConfig` 新增 `capture_commit_action: "toolbar" | "editor"`，用 `#[serde(default)]`
     加入，默认 `"editor"`（无需配置迁移，空值即默认）；设置页截图区加一个选择项。
   - `react/capture-overlay/App.tsx`：`selection.pointerUp` 产生有效选区且配置为 `editor` 时
     直接 `run("edit")`；`toolbar` 保持现状。快捷路径不丢：`Enter` 仍是"复制并结束"
     （`App.tsx:120-128` 已有），浮动工具条在 `editor` 模式下不再等待点击。
   - 覆盖层的 `windows` 悬停选窗、`Esc` 取消、选区翻译都不动。
   - 实施时的一处补充（方案里没写）：`editor` 模式下松手前按住 `Alt` 会临时留在工具条上。
     编辑器窗口没有翻译入口，若不留这个出口，默认配置会让"选区翻译"整个功能没有入口。
2. **编辑器窗口尺寸随图像**（这才是"没有正确缩放"的真因）：
   - `commands/capture_editor.rs:63-70`：用已经在手的 `width/height`
     （`queue_capture_for_editor` 的入参）算逻辑尺寸 = 物理尺寸 / `scale_factor`，
     加上侧栏与 chrome 的固定占位（参考 flashot Pin 的 padding 思路），
     经 `WorkArea::clamp_size` 收敛，再 `min_inner_size(820,560)` 兜底；仍然 `center()`。
     复用窗口时（`window.navigate` 分支）同样重设尺寸。纯计算部分抽成
     `fn editor_window_size(image: (u32,u32), scale: f64, work: WorkArea) -> (f64,f64)` 加单测。
   - `react/capture/captureViewport.ts:33`：`fitScale` 去掉 `, 1` 的钳制，改成
     `clamp(min(maxW/nw, maxH/nh), 1, MAX_FIT_UPSCALE=3)` 的上采样上限——窗口已经贴合图像，
     小选区不再是大窗里的一小块；`buildViewport` 已有单测文件
     （`src/tests/capture-editor-*.test.js`），补 3 条断言（小图上采样、大图缩小、上限钳制）。
3. **悬停窗口高亮的可发现性**：flashot 也不显示标题，所以不加标题标签；但补两点小改：
   - 已有选区后仍允许悬停预览其它窗口（现在 `App.tsx:150` 是 `!selected ? candidate : null`，
     有选区后就不再提示，用户会以为选窗功能没了）；
   - `window_probe.rs` 在 `xcap::Window::all()` 失败或返回空时记一条 `log::info`，
     并在覆盖层顶部给一行提示（英文 UI 文案）"Window picking unavailable in this session"，
     让 Wayland 下的退化可见而不是像坏了。
4. **不做**：滚动截图（Phase 4 已判定不做，理由未变）、Board 模式、原地 Konva 标注、
   Pin 落在选区原位（Clippy 的 Pin 有自己的尺寸/位置策略，改动面大于收益，先留档）。

---

## 4. 自动化能覆盖到哪里

| 改动 | 自动化验证 | 需人工 |
|---|---|---|
| 键盘状态机（修 2/3） | `keyboard-router.test.js` 全覆盖（纯函数 + 分派） | 手感确认 |
| 失焦豁免（修 4-1） | Rust 纯函数单测 | 真机点下拉框 |
| custom-select 分组/刷新（修 4-2） | `custom-select.test.js`、`codec.test.js` | 视觉 |
| 翻译区布局（修 1） | 新增 headless Firefox 布局 smoke | dev 服务器看实际观感 |
| 窗口尺寸（修 1-3、修 5-2） | `window_controller.rs` / `capture_editor.rs` 单测 | HiDPI 真机 |
| fitScale 上采样（修 5-2） | `capture-editor-*.test.js` | 视觉 |
| 提交即进编辑器（修 5-1） | 覆盖层交互单测 + 配置默认值测试 | 真机截图全流程 |

## 5. 建议执行顺序

1. 修 4-1 + 修 2/3（状态机）——最小改动、纯逻辑、全可单测，先把"按键乱跑、窗口消失"止血。
2. 修 1（布局 + 窗口高度 + 布局 smoke）——需要起 dev 服务器边看边调。
3. 修 4-2（换掉原生 select，含 custom-select 补能力）。
4. 修 5-2（窗口尺寸 + fitScale）→ 修 5-1（提交即进编辑器 + 设置项）→ 修 5-3（提示与退化可见）。
5. 全量门禁 `./scripts/ci-local.sh`，更新 `docs/architecture.md`（新增"主窗口键盘状态机"一节）、
   `qa-matrix.md`（真机行：codec 下拉框不再丢窗口、Tab 下 ws 仍走列表、截图提交即进编辑器）。
