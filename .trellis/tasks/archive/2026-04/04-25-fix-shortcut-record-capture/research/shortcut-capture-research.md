# Research: 全局快捷键录制 — KeyboardEvent 捕获与 Tauri v2 适配

- **Query**: 浏览器/Tauri webview 中如何稳定录制全局快捷键，并转成 Tauri 快捷键字符串
- **Scope**: 外部（MDN / Tauri 官方文档 / Chromium 行为）
- **Date**: 2026-04-25

---

## 1. `KeyboardEvent.key` vs `KeyboardEvent.code`

### 核心差异

| 属性 | 含义 | 受影响因素 | 示例（按物理 V 键） |
|---|---|---|---|
| `event.key` | 经过操作系统/输入法/键盘布局映射后**最终产生的字符**（或命名键） | 键盘布局（QWERTY/AZERTY/Dvorak）、输入法（IME）、修饰键（Shift/AltGr）、Dead Key、CapsLock | `"v"` / `"V"`（Shift）/ `"√"`（macOS Option+V）/ `"‌"`（AltGr 组合）/ `"Dead"` |
| `event.code` | 物理键的标识符，**与布局/输入法无关** | 仅与键盘硬件规范（USB HID）相关 | 始终为 `"KeyV"` |

### 在快捷键录制场景下 `key` 的常见坑

1. **大小写不一致**：按住 Shift 时 `key` 变成大写字母（`"V"`），未按 Shift 时是小写（`"v"`），需要手动 `toUpperCase()` 才能稳定。
2. **AltGr / Option 组合产生特殊字符**：
   - macOS：`Option + V` → `key === "√"`（数学根号）；`Option + N` → 触发 dead key（`key === "Dead"`）然后等待下一个键。
   - Windows/Linux 国际键盘：`AltGr + Q` → `key === "@"`（部分布局）。
   - 这会让"按 Alt + V"在不同机器上录到的 `key` 完全不同。
3. **非 US 布局**：法语 AZERTY 上物理 `KeyQ` 输入的是 `"a"`；中文 IME 激活时 `key` 可能是 `"Process"` 或空字符串。
4. **Dead key**：欧洲重音键、中文 IME composition 期间 `key` 可能是 `"Dead"`、`"Process"`、`"Unidentified"`。
5. **AltGraph**：在某些 Linux 布局下，按下 AltGr 自身会产生 `key === "AltGraph"`，但实际上这通常是 Ctrl+Alt 的合成 — 录制时容易把组合误判成普通键。

### `code` 的优势与边界

- **稳定**：无论用户在哪种布局/IME 下，`KeyV` 永远是 V 物理键、`Digit1` 永远是数字行 1。
- **限制**：
  - 不能区分国际键的逻辑符号（用户想录"输入 @ 的键"时 `code` 不直观，但快捷键场景下我们要的就是物理位置）。
  - 部分 OEM 键 `code` 可能是 `IntlBackslash`、`IntlRo`、`IntlYen` 等不在常见映射表中。
  - 数字小键盘在 NumLock 关闭时仍是 `Numpad1`（`key` 反而变成 `"End"`）。

### 推荐做法

> 录制阶段统一用 `e.code` 解析"物理键"，再通过映射表转 Tauri 命名；只在 `code` 落到非映射区时回退到 `e.key.toUpperCase()`。

---

## 2. `code` → Tauri 快捷键字符串映射

Tauri v2 `tauri-plugin-global-shortcut` 内部用 [`global-hotkey`](https://docs.rs/global-hotkey/) crate，accelerator 字符串语法与 Electron `globalShortcut` / Chromium `commands` 类似：

```
<Modifier>+<Modifier>+...+<Key>
```

- 修饰键名：`CommandOrControl` / `CmdOrCtrl` / `Control` / `Ctrl` / `Alt` / `Option` / `AltGr` / `Shift` / `Super` / `Meta`
- 主键名：单字母大写 `A-Z`、数字 `0-9`、`F1-F24`、`Space`、`Tab`、`Enter`、`Escape`、`Backspace`、`Delete`、`Insert`、`Home`、`End`、`PageUp`、`PageDown`、`Up/Down/Left/Right`（也接受 `ArrowUp` 等）、`Plus`、`Minus`、`Comma`、`Period`、`Slash`、`Backslash`、`Semicolon`、`Quote`、`BracketLeft/Right`、`Backquote`、`Equal`，以及 `Numpad0-9`、`NumpadAdd` 等。

### 推荐映射表（节选）

| `e.code` | Tauri key |
|---|---|
| `KeyA` … `KeyZ` | `A` … `Z` |
| `Digit0` … `Digit9` | `0` … `9` |
| `Numpad0` … `Numpad9` | `Num0` … `Num9`（Tauri 文档使用 `Num0` 命名；如果失败回退 `Numpad0`） |
| `F1` … `F24` | `F1` … `F24` |
| `Space` | `Space` |
| `Tab` | `Tab` |
| `Enter` / `NumpadEnter` | `Enter` |
| `Escape` | `Escape` |
| `Backspace` | `Backspace` |
| `Delete` | `Delete` |
| `Insert` | `Insert` |
| `Home` / `End` | `Home` / `End` |
| `PageUp` / `PageDown` | `PageUp` / `PageDown` |
| `ArrowUp/Down/Left/Right` | `Up`/`Down`/`Left`/`Right` |
| `Minus` | `-`（或 `Minus`） |
| `Equal` | `=`（或 `Plus`） |
| `BracketLeft` / `BracketRight` | `[` / `]` |
| `Backslash` | `\` |
| `Semicolon` / `Quote` | `;` / `'` |
| `Comma` / `Period` / `Slash` | `,` / `.` / `/` |
| `Backquote` | `` ` `` |

### 修饰键映射

```text
e.ctrlKey   → "CmdOrCtrl"   // 跨平台推荐；如果想强制 Ctrl，用 "Ctrl"
e.metaKey   → "CmdOrCtrl"   // macOS 上的 ⌘
e.altKey    → "Alt"
e.shiftKey  → "Shift"
```

> 我们项目目标是 Linux，可以直接 `e.ctrlKey → "Ctrl"`、`e.metaKey → "Super"`，无需 `CmdOrCtrl` 抽象。

---

## 3. 浏览器中"高优先级"捕获键盘事件

### capture 阶段注册

DOM 事件分三阶段：capture（从 window 向下）→ target → bubble（向上）。默认监听器在 bubble 阶段触发；用 `{ capture: true }` 可在事件最早期就拿到：

```js
const handler = (e) => { /* ... */ };
window.addEventListener('keydown', handler, { capture: true });
// 注销时也必须带 capture: true
window.removeEventListener('keydown', handler, { capture: true });
```

> capture 监听器优先于同一目标上后注册的 bubble 监听器，但**不能跨过比自己更早注册的同级 capture 监听器**——所以"先到先得"。在录制窗口打开时立即注册即可。

### 阻止默认与其他监听器

```js
e.preventDefault();         // 阻止浏览器默认行为（如 Ctrl+L 聚焦地址栏 — 在 webview 里通常不存在，但 Ctrl+W/Ctrl+R 仍可能被 Tauri/系统接管）
e.stopImmediatePropagation(); // 既阻止冒泡，又阻止同元素上后续监听器执行
// 注意：stopPropagation() 只阻止冒泡，不阻止同元素其它监听器
```

### `setTimeout(stopRecording, 0)` 模式

录制成功后，立刻在同一 tick 里 `unregister/await invoke(...)` 容易出现：
- 当前 keydown 还在派发链上，异步 IPC resolve 后再操作 DOM/state 会造成"录制器先关闭、之后系统快捷键又触发一次"的双触发。
- 使用 `setTimeout(() => stopRecording(), 0)` 把清理推到下一个事件循环 tick，等当前 keydown 完全冒泡完、preventDefault 已生效、再去注销监听 / 重新注册 Tauri 快捷键，避免重入。

### 浏览器/系统拦不住的组合

录制时要给用户提示：以下组合 webview 层无法拦截，应让用户换：
- macOS：`Cmd+Q`、`Cmd+H`、`Cmd+M`、`Cmd+Tab`、`Cmd+Space`（Spotlight）
- Windows：`Ctrl+Alt+Del`、`Win+L`、`Win+D`、`Alt+Tab`、`Alt+F4`（部分可拦但不稳定）
- Linux/GNOME：`Super` 系（活动总览）、`Ctrl+Alt+T`（终端，依赖 WM 配置）、`Ctrl+Alt+F1-F7`（切换 TTY）
- 通用：`PrintScreen`、媒体键在某些 OS 上被驱动层吞掉

实践上：**录到这些组合时弹一个 toast 让用户重选**，不要把它们写入配置。

---

## 4. Tauri v2 `tauri-plugin-global-shortcut` 录制期注意事项

参考 https://v2.tauri.app/plugin/global-shortcut/ 与 docs.rs 上的 `tauri_plugin_global_shortcut`：

- `app.global_shortcut().register("Ctrl+Alt+V")` 注册单个 accelerator；`register_multiple` 批量注册。
- `unregister("Ctrl+Alt+V")` 注销单个；`unregister_all()` 注销当前进程注册的所有。
- 插件初始化时通过 `Builder::new().with_handler(|app, shortcut, event| { ... }).build()` 配置统一回调；事件包含 `state: Pressed | Released`，**默认会触发两次**，业务里要过滤 `state == Pressed`。
- accelerator 字符串中的 key 名大小写不敏感，但用空格/`+` 分隔修饰键。

### 录制期间的双触发问题

> 场景：用户当前热键是 `Ctrl+Alt+V`；他在设置页里按这同一组合想录制新的 — 此时 Tauri 的 global-shortcut 还在监听，按下瞬间会**同时**触发：
> 1. 全局热键回调（弹出/隐藏主窗口）
> 2. 设置页里的 `keydown` capture 监听器（录制）

### 推荐流程

```text
打开录制弹窗
  ├─ await invoke('pause_global_shortcuts')   // 后端调用 unregister_all()
  └─ window.addEventListener('keydown', recorder, { capture: true })

捕获到合法组合
  ├─ e.preventDefault(); e.stopImmediatePropagation();
  ├─ const accelerator = keyEventToShortcut(e);
  ├─ setTimeout(() => {
  │     window.removeEventListener('keydown', recorder, { capture: true });
  │     invoke('resume_global_shortcuts', { newShortcut: accelerator });
  │     // 后端：先 unregister_all (幂等)，再用新配置 register
  │  }, 0);
```

### 后端命令草稿

```rust
#[tauri::command]
fn pause_global_shortcuts(app: tauri::AppHandle) -> Result<(), String> {
    app.global_shortcut().unregister_all().map_err(|e| e.to_string())
}

#[tauri::command]
fn resume_global_shortcuts(app: tauri::AppHandle, new_shortcut: String) -> Result<(), String> {
    let gs = app.global_shortcut();
    gs.unregister_all().ok();              // 幂等清理
    gs.register(new_shortcut.as_str()).map_err(|e| e.to_string())
}
```

> 文档原句（节选）："Use `unregister_all` to remove all shortcuts that have been registered by your application." — Tauri v2 plugin global-shortcut。

---

## 5. 推荐的 `keyEventToShortcut(e)` 实现思路（伪代码）

```js
// ---- 常量映射 ----
const MOD_KEYS = new Set([
  'Control', 'ControlLeft', 'ControlRight',
  'Alt', 'AltLeft', 'AltRight', 'AltGraph',
  'Shift', 'ShiftLeft', 'ShiftRight',
  'Meta', 'MetaLeft', 'MetaRight', 'OS',
  'CapsLock', 'NumLock', 'ScrollLock',
]);

const CODE_TO_KEY = {
  // 字母
  ...Object.fromEntries('ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('').map(c => [`Key${c}`, c])),
  // 数字行
  ...Object.fromEntries('0123456789'.split('').map(d => [`Digit${d}`, d])),
  // 小键盘
  ...Object.fromEntries('0123456789'.split('').map(d => [`Numpad${d}`, `Num${d}`])),
  NumpadAdd: 'NumAdd', NumpadSubtract: 'NumSub',
  NumpadMultiply: 'NumMult', NumpadDivide: 'NumDiv',
  NumpadDecimal: 'NumDec', NumpadEnter: 'Enter',
  // 功能键
  ...Object.fromEntries(Array.from({ length: 24 }, (_, i) => [`F${i + 1}`, `F${i + 1}`])),
  // 命名键
  Space: 'Space', Tab: 'Tab', Enter: 'Enter', Escape: 'Escape',
  Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
  ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  // 标点
  Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
  Backslash: '\\', Semicolon: ';', Quote: "'",
  Comma: ',', Period: '.', Slash: '/', Backquote: '`',
};

// ---- 主函数 ----
function keyEventToShortcut(e) {
  // 1. 修饰键自身按下时不算结束键
  if (MOD_KEYS.has(e.code) || MOD_KEYS.has(e.key)) return null;

  // 2. composing / dead key / IME 激活 — 跳过
  if (e.isComposing || e.key === 'Dead' || e.key === 'Process' || e.key === 'Unidentified') return null;

  // 3. 收集修饰键（至少要有一个，否则不接受单键热键）
  const mods = [];
  if (e.ctrlKey)  mods.push('Ctrl');
  if (e.altKey)   mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey)  mods.push('Super');   // Linux/Win 的 Super；macOS 上是 ⌘ — 按需改 'Cmd' 或 'CmdOrCtrl'
  if (mods.length === 0) return null;   // 拒绝纯字母/F 键单键热键，避免误触

  // 4. 主键：先查 code 表，否则回退 key
  let mainKey = CODE_TO_KEY[e.code];
  if (!mainKey) {
    const fallback = (e.key || '').toUpperCase();
    if (!fallback || fallback.length > 3) return null;  // 过滤 'ARROWUP' 这种长串 — 但其实上面 ArrowUp 已映射；保险
    mainKey = fallback;
  }

  // 5. 组装 Tauri accelerator
  return [...mods, mainKey].join('+');
}
```

### 边界 case 校验

- **只按 Shift+A**：`ctrlKey/altKey/metaKey` 都是 false → 返回 null（避免把 `Shift+字母` 当快捷键，因为很多 IME/输入会触发）。如果产品上要允许 `Shift+F5` 这类，把校验改成"必须有 Ctrl/Alt/Meta 之一，或主键是 F1-F24"。
- **Ctrl+Alt+V（Linux 法语布局）**：`e.code === 'KeyV'` 命中映射 → `Ctrl+Alt+V`，不受 `e.key === '◊'` 之类干扰。
- **Ctrl+Numpad1**（NumLock 关）：`e.code === 'Numpad1'` 命中 → `Ctrl+Num1`，不会变成 `Ctrl+End`。
- **AltGr+某键**（Linux）：浏览器把 AltGr 报成 `ctrlKey + altKey` 同时为 true（GTK webview 行为），可能误录成 `Ctrl+Alt+...`。可在 UI 提示用户避免使用 AltGr，或检查 `e.getModifierState('AltGraph')` 单独剔除。
- **IntlBackslash / IntlRo**：不在映射表 → 走 `e.key.toUpperCase()` 回退，可能得到 `<` 或 `\`，需要用户自行验证 Tauri 是否接受；不接受时报错让用户重录。

---

## Caveats / Not Found

- 未直接抓取 Tauri v2 plugin global-shortcut 文档原文 URL（本次未联网）— 上文 API 描述基于已知的 `tauri-plugin-global-shortcut` v2 公共接口（`register` / `unregister` / `unregister_all` / `with_handler`）。落地前请以 `https://v2.tauri.app/plugin/global-shortcut/` 与 `docs.rs/tauri-plugin-global-shortcut` 当前版本为准核对方法签名（特别是 `state: Pressed | Released` 的过滤）。
- Tauri accelerator 字符串中 `Numpad0` 与 `Num0` 的命名兼容性在不同插件版本间略有变化，建议运行时跑一遍 `register("Ctrl+Num0")` 与 `register("Ctrl+Numpad0")` 验证。
- macOS 行为（`Option` 产生特殊字符、`Cmd` 拦不住）项目目前只构建 Linux，可在第一版录制器里把 `metaKey` 直接映射为 `Super`，跨平台时再补充。
