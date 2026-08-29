/**
 * shortcut-recorder.js — 把 KeyboardEvent 转成 Tauri 快捷键字符串。
 *
 * 设计要点：
 *   - 主键来自 e.code，避免不同键盘布局/输入法/AltGr 让 e.key 变成特殊字符
 *     （如 macOS Option+V → "√"，AZERTY 下 KeyV 的 key 可能是 ‹ 等）。
 *   - 至少要求一个非 Shift 修饰键（Ctrl / Alt / Meta），避免单键热键和
 *     "Shift+字母"误录。
 *   - 修饰键名严格使用 global-hotkey crate 的解析器可识别的名称：
 *     Ctrl / Alt / Shift / Super。不使用 CmdOrCtrl（Linux 无歧义）。
 */

const MOD_CODES = new Set([
  "ControlLeft", "ControlRight",
  "AltLeft", "AltRight", "AltGraph",
  "ShiftLeft", "ShiftRight",
  "MetaLeft", "MetaRight", "OSLeft", "OSRight",
  "CapsLock", "NumLock", "ScrollLock",
]);
const MOD_KEYS = new Set(["Control", "Alt", "AltGraph", "Shift", "Meta", "OS"]);

const CODE_TO_KEY = (() => {
  const map = {};
  for (const c of "ABCDEFGHIJKLMNOPQRSTUVWXYZ") map[`Key${c}`] = c;
  for (const d of "0123456789") {
    map[`Digit${d}`] = d;
    map[`Numpad${d}`] = `Num${d}`;
  }
  for (let i = 1; i <= 24; i++) map[`F${i}`] = `F${i}`;
  Object.assign(map, {
    Space: "Space", Tab: "Tab", Enter: "Enter", NumpadEnter: "Enter",
    Escape: "Escape", Backspace: "Backspace", Delete: "Delete", Insert: "Insert",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Backslash: "\\", Semicolon: ";", Quote: "'",
    Comma: ",", Period: ".", Slash: "/", Backquote: "`",
    NumpadAdd: "NumAdd", NumpadSubtract: "NumSub",
    NumpadMultiply: "NumMult", NumpadDivide: "NumDiv",
    NumpadDecimal: "NumDec",
  });
  return map;
})();

export function keyEventToShortcut(e) {
  if (MOD_CODES.has(e.code) || MOD_KEYS.has(e.key)) return null;
  if (e.isComposing || e.key === "Dead" || e.key === "Process" || e.key === "Unidentified") {
    return null;
  }

  const modifiers = [];
  if (e.ctrlKey)  modifiers.push("Ctrl");
  if (e.altKey)   modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey)  modifiers.push("Super");

  const hasNonShift = modifiers.some((m) => m !== "Shift");
  if (!hasNonShift) return null;

  let mainKey = CODE_TO_KEY[e.code];
  if (!mainKey) {
    const fallback = (e.key || "").toUpperCase();
    if (!fallback || fallback.length > 3) return null;
    mainKey = fallback;
  }

  return modifiers.concat(mainKey).join("+");
}

const MOD_ALIASES = {
  ctrl: "Ctrl", control: "Ctrl", cmdorctrl: "Ctrl", commandorcontrol: "Ctrl",
  alt: "Alt", option: "Alt",
  shift: "Shift",
  super: "Super", meta: "Super", cmd: "Super", command: "Super", win: "Super",
};
const MOD_ORDER = ["Ctrl", "Alt", "Shift", "Super"];

/**
 * 归一化快捷键字符串，用于比较两个组合是否是同一个键位：
 * 修饰键顺序、别名（Control/Meta…）与主键大小写都不参与比较。
 * 空串或只有修饰键返回空串，表示"没有可比较的键位"。
 */
export function normalizeShortcut(shortcut) {
  const parts = String(shortcut ?? "").split("+").map((part) => part.trim()).filter(Boolean);
  if (!parts.length) return "";
  const modifiers = new Set();
  const keys = [];
  for (const part of parts) {
    const alias = MOD_ALIASES[part.toLowerCase()];
    alias ? modifiers.add(alias) : keys.push(part.toUpperCase());
  }
  if (!keys.length) return "";
  return MOD_ORDER.filter((mod) => modifiers.has(mod)).concat(keys).join("+");
}
