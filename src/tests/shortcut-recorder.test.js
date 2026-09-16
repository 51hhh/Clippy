import { describe, it, expect } from "vitest";
import { keyEventToShortcut, normalizeShortcut } from "../js/shortcut-recorder.js";

function ev(overrides) {
  return {
    code: "",
    key: "",
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    metaKey: false,
    isComposing: false,
    ...overrides,
  };
}

describe("keyEventToShortcut", () => {
  it("Ctrl+Alt+V (Linux QWERTY)", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "v", ctrlKey: true, altKey: true })),
    ).toBe("Ctrl+Alt+V");
  });

  it("Ctrl+Alt+V 即使 e.key 是 macOS Option+V 的特殊字符也走 code", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "√", ctrlKey: true, altKey: true })),
    ).toBe("Ctrl+Alt+V");
  });

  it("AZERTY 下 KeyV 物理键，e.key 可能是非 'V' 字符，仍正确解析", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "◊", ctrlKey: true, altKey: true })),
    ).toBe("Ctrl+Alt+V");
  });

  it("Ctrl+Shift+Digit1 → Ctrl+Shift+1", () => {
    expect(
      keyEventToShortcut(ev({ code: "Digit1", key: "!", ctrlKey: true, shiftKey: true })),
    ).toBe("Ctrl+Shift+1");
  });

  it("Ctrl+F5", () => {
    expect(
      keyEventToShortcut(ev({ code: "F5", key: "F5", ctrlKey: true })),
    ).toBe("Ctrl+F5");
  });

  it("Ctrl+Numpad1 (NumLock 关时 e.key=End 也不影响)", () => {
    expect(
      keyEventToShortcut(ev({ code: "Numpad1", key: "End", ctrlKey: true })),
    ).toBe("Ctrl+Num1");
  });

  it("Super+Space", () => {
    expect(
      keyEventToShortcut(ev({ code: "Space", key: " ", metaKey: true })),
    ).toBe("Super+Space");
  });

  it("macOS 可以用 Command 语义展示 Meta 键", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "v", metaKey: true }), "Command"),
    ).toBe("Command+V");
    expect(normalizeShortcut("Command+V")).toBe(normalizeShortcut("Super+V"));
  });

  it("Ctrl+ArrowUp → Ctrl+Up", () => {
    expect(
      keyEventToShortcut(ev({ code: "ArrowUp", key: "ArrowUp", ctrlKey: true })),
    ).toBe("Ctrl+Up");
  });

  it("修饰键自身按下返回 null", () => {
    expect(
      keyEventToShortcut(ev({ code: "ControlLeft", key: "Control", ctrlKey: true })),
    ).toBeNull();
    expect(
      keyEventToShortcut(ev({ code: "AltRight", key: "Alt", altKey: true })),
    ).toBeNull();
  });

  it("纯字母键无修饰返回 null", () => {
    expect(keyEventToShortcut(ev({ code: "KeyA", key: "a" }))).toBeNull();
  });

  it("仅 Shift+字母 也返回 null", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyA", key: "A", shiftKey: true })),
    ).toBeNull();
  });

  it("IME composing 期间返回 null", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "Process", ctrlKey: true, altKey: true, isComposing: true })),
    ).toBeNull();
  });

  it("Dead key 返回 null", () => {
    expect(
      keyEventToShortcut(ev({ code: "KeyV", key: "Dead", ctrlKey: true, altKey: true })),
    ).toBeNull();
  });

  it("非映射 code 走 e.key.toUpperCase 回退", () => {
    expect(
      keyEventToShortcut(ev({ code: "IntlBackslash", key: "<", ctrlKey: true })),
    ).toBe("Ctrl+<");
  });

  it("回退也无效时返回 null", () => {
    expect(
      keyEventToShortcut(ev({ code: "Unidentified", key: "", ctrlKey: true })),
    ).toBeNull();
  });
});

describe("normalizeShortcut", () => {
  it("忽略修饰键顺序、别名与主键大小写", () => {
    expect(normalizeShortcut("Alt+Ctrl+v")).toBe("Ctrl+Alt+V");
    expect(normalizeShortcut("Control+Alt+V")).toBe("Ctrl+Alt+V");
    expect(normalizeShortcut("Meta+v")).toBe("Super+V");
    expect(normalizeShortcut("CmdOrCtrl+Shift+s")).toBe("Ctrl+Shift+S");
  });

  it("空值与只有修饰键的输入没有可比较的键位", () => {
    expect(normalizeShortcut("")).toBe("");
    expect(normalizeShortcut(null)).toBe("");
    expect(normalizeShortcut("Ctrl+Alt")).toBe("");
  });

  it("同一键位的不同写法归一化后相等", () => {
    expect(normalizeShortcut("super+V")).toBe(normalizeShortcut("Meta+v"));
    expect(normalizeShortcut("Ctrl+2")).not.toBe(normalizeShortcut("Ctrl+3"));
  });
});
