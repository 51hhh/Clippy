# Clipboard panel redesign

- Date: 2026-04-25
- Status: Approved

## 目标

主窗口（380×500 浮动面板）的剪贴板列表 UI 与交互重做：默认零干扰，鼠标/键盘双轨语义一致，结合主题营造现代极简清爽风。

## 默认布局（零干扰）

```
┌──────────────────────────────────────────────┐
│  ▾ Lorem ipsum dolor sit amet, conse…   ⋯  │  ← 焦点行（左 stripe + ▾ 指示）
│    2 min · Text · 124 B                       │
│  ─────────────────────────────────────────── │
│    git status                            ⋯  │
│    yesterday · Shell · 12 B                   │
│  ─────────────────────────────────────────── │
│    [thumbnail]                           ⋯  │
│    3 days · Image · 89 KB                     │
│                                               │
│              ─────                            │
│            All 142 · Favorites 12             │  ← 底部 segment
└──────────────────────────────────────────────┘
```

- 无顶部 header / 搜索 / tabs
- 默认窗口出现时焦点落在第 1 条
- 每行右侧只 `⋯`；收藏项预览前置 `★`（不占右侧空间）
- 当前焦点行用左 accent stripe + `▾` 指示器
- 底部 1 行 segment：`All N · Favorites M`，带实时计数

## 召唤搜索

- 焦点在**第 1 条**时，按 `↑` 或 `W`：顶部滑入迷你搜索条并自动聚焦
- 搜索条：左 🔍、输入框、右侧 `Esc` 提示
- `Esc` 三段：清空 → 收起搜索条 → 隐藏面板
- 鼠标：上边缘悬浮提示带可点展开

## 切换 All / Favorites

- 在任意位置按 `←` 或 `A`：滑入"Favorites"分栏（左→右滑动过渡）
- `→` 或 `D`（且当前焦点在行体而非按钮区时）：滑回"All"
- 底部 segment 同步高亮
- 鼠标：直接点底部 segment 切换

## 行内操作（每行只 `⋯`，按钮默认隐藏）

```
default :│ Lorem ipsum dolor sit…       ⋯ │
active  :│ Lorem ip…   [⎘]  [☆]  [✕]  ⋯ │
```

- 点击 `⋯` 或键盘 `→`/`D`（焦点在行体时）：按钮组从 `⋯` 左侧依次展开 [⎘ Copy] [☆ Favorite] [✕ Delete]，覆盖预览右侧
- 一次只有一行处于"展开态"
- 收回：再点 `⋯`、点行外、复制成功、`Esc`、或在最左按钮再按 `←`/`A`
- 整行点击（非按钮区）= 复制并隐藏（最高频路径）
- 删除二次确认：`✕` 第一次变成 `Confirm?` 1.2 秒，1.2 秒内再次激活才真正删除

## 双轨操作语义

| 操作 | 鼠标 | 键盘 |
|---|---|---|
| 选行 | hover | `↑↓` / `W S` |
| 复制 + 隐藏 | 单击行体 | 焦点在行体时 `Space` / `Enter` |
| 展开按钮组 | 点 `⋯` | 焦点在行体时 `→` / `D` |
| 按钮间移动 | 鼠标移动 | `← →` / `A D` |
| 执行按钮 | 单击按钮 | 焦点在按钮上时 `Space` / `Enter` |
| 收回按钮组 | 点 `⋯`/行外/复制 | `Esc`，或最左按钮再 `←`/`A` |
| 切到 Favorites | 点 segment | 焦点在行体时 `←` / `A` |
| 切回 All | 点 segment | 焦点在行体时 `→` / `D` |
| 召唤搜索 | 上边缘条 | 焦点在第 1 行时 `↑` / `W` |
| 隐藏面板 | 失焦 | `Esc` 三段后 |

注：`←/A`、`→/D` 在「行体焦点」是 tab 切换，在「按钮焦点」是按钮间移动；不同场景下含义清晰，由 `focusedAction` 状态决定。

## 状态机

模块状态：
```
panelMode  : "all" | "favorites"           // 横向 tab
focusedRow : number                         // 当前光标行索引
expandedRow: number | null                  // 当前展开操作组的行
focusedCol : -1 | 0 | 1 | 2                // -1=行体, 0..2=按钮 [Copy, Favorite, Delete]
deletePending: { rowId, expiresAt } | null  // 删除二次确认
```

## 模块边界

- `index.html`：去 header/tabs 旧 DOM；保留 `<main id="clip-list">`，加 `<aside id="search-bar" hidden>`、`<footer id="segment-tabs">`
- `js/clipboard-list.js`：扩展为状态机宿主，导出
  ```js
  init, refresh,
  moveRow(delta), moveCol(delta),
  expandRowActions(), collapseRowActions(),
  activateFocus(),  // 复制 / 执行按钮 / 切 tab 的统一入口
  setPanelMode(mode),
  ```
- `js/search-bar.js`（新）：搜索条显隐 + Esc 三段；接受 `onQuery(q)` 回调
- `js/segment-tabs.js`（新）：底部 segment 渲染 + 计数 + 点击切换
- `js/app.js`：键盘路由器集中，根据 focused 状态分发
- `styles/components.css`：重写为 modules — `clip-list / clip-row / row-stripe / row-actions / segment-tabs / search-bar`，行为和颜色全走 `var(--*)`
- 仍走 `api.js` 调 `selectClip / toggleFavorite / deleteClip`

## 视觉细节

- 行 padding 8/12，行高约 52 px（两行时），全行可点
- 焦点行左侧 3 px accent stripe（`var(--accent)`），透明度 1
- 按钮组按钮：32×28、圆角 6、`bg-secondary` 背景、`text-primary` 图标，hover 变 `accent-soft`
- `Confirm?` 状态：按钮文字红色 `var(--danger)`，背景 `accent-soft`
- 切 tab 动画：`transform: translateX(-100%)` 与 0 间过渡 180 ms ease
- 搜索条滑入：`max-height` + `opacity` 200 ms ease
- 6 套主题在浅色和深色背景下对比 ≥ 4.5:1（已在 themes.css 验证）

## 可观测埋点

新增事件（沿用 `js/telemetry.js`）：
- `clip-list:focus-row { idx, mode }`
- `clip-list:expand-actions { idx }`
- `clip-list:collapse-actions { idx }`
- `clip-list:invoke-action { action: "copy"|"favorite"|"delete", source: "mouse"|"keyboard" }`
- `clip-list:set-mode { mode: "all"|"favorites" }`
- `search-bar:summon { source: "mouse"|"keyboard" }`
- `search-bar:dismiss { stage: "clear"|"hide"|"panel" }`

## 测试

vitest + jsdom：

- `tests/clipboard-list-focus.test.js`
  - 默认 init 后 focusedRow=0
  - 在 row 0 ↑ → search-bar 召唤事件
  - ↓ 向下移行；到底再 ↓ 不溢出
  - row 焦点上 → 展开按钮组；← 在按钮组最左收回；Esc 收回
  - row 焦点 Space = 复制（调 api.selectClip）
- `tests/clipboard-list-actions.test.js`
  - 点 `⋯` 出按钮组；点按钮调 api；点行外收回
  - 删除二次确认：第一次按 ✕ 不调 deleteClip；1.2s 内第二次才调
- `tests/segment-tabs.test.js`
  - 计数随数据更新
  - 点 Favorites segment → setPanelMode("favorites")
- `tests/search-bar.test.js`
  - summon → 显示并聚焦
  - 输入触发 onQuery（防抖）
  - Esc 三段顺序

## 错误处理

- `selectClip / toggleFavorite / deleteClip` 失败：`console.error` + `telemetry.emit('clip-list:invoke-action-error', {...})`，UI 不解锁按钮组
- 删除二次确认超时未点：自动还原按钮文字
- 切换 tab 时如果 favorites 列表为空，显示空状态（"No favorites yet"）

## 不做（YAGNI）

- 拖拽排序、批量选择、导出、HTML 富文本渲染
- 翻页/分页加载（保留现有 200 条上限即可）
- 自定义快捷键改键
- 搜索高亮匹配项

## 影响面

- 改：`src/index.html`、`src/js/clipboard-list.js`、`src/js/app.js`、`src/styles/components.css`、`src/i18n/i18n.js`（新增收藏空状态文案 + segment 文案 + Esc 提示）
- 新：`src/js/search-bar.js`、`src/js/segment-tabs.js`、`src/tests/*.test.js`
- 不动后端

## 验收

- 6 套主题视觉过一遍
- 380×500 默认尺寸下所有按钮、文字不被裁
- `cd src && npm test` / `npx vite build` / `cargo tauri build` 全过
- 手动：默认打开第 1 条已焦点；↑ 召唤搜索；←/→ 切 tab；→ 展开按钮；空格执行
