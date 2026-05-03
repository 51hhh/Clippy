import { describe, it, expect, beforeEach } from "vitest";
import { initCustomSelect } from "../js/custom-select.js";

/** 构造自定义下拉框 DOM */
function setup(selectedValue = "a") {
  document.body.innerHTML = `
    <div class="custom-select" id="cs">
      <div class="custom-select-trigger" tabindex="0">
        <span class="custom-select-value">A</span>
        <span class="custom-select-arrow">▾</span>
      </div>
      <ul class="custom-select-dropdown">
        <li class="custom-select-option${selectedValue === "a" ? " selected" : ""}" data-value="a">A</li>
        <li class="custom-select-option${selectedValue === "b" ? " selected" : ""}" data-value="b">B</li>
        <li class="custom-select-option${selectedValue === "c" ? " selected" : ""}" data-value="c">C</li>
      </ul>
    </div>
  `;
  const container = document.getElementById("cs");
  const ctrl = initCustomSelect(container);
  const trigger = container.querySelector(".custom-select-trigger");
  return { container, ctrl, trigger };
}

describe("custom-select", () => {
  beforeEach(() => { document.body.innerHTML = ""; });

  it("初始值从 .selected 选项读取", () => {
    const { ctrl } = setup("b");
    expect(ctrl.value).toBe("b");
  });

  it("点击 trigger 展开/收起", () => {
    const { container, trigger } = setup();
    expect(container.classList.contains("open")).toBe(false);
    trigger.click();
    expect(container.classList.contains("open")).toBe(true);
    trigger.click();
    expect(container.classList.contains("open")).toBe(false);
  });

  it("点击选项更新值并关闭下拉", () => {
    const { container, ctrl, trigger } = setup();
    trigger.click();
    container.querySelector('[data-value="c"]').click();
    expect(ctrl.value).toBe("c");
    expect(container.classList.contains("open")).toBe(false);
  });

  it("编程设值更新显示文本", () => {
    const { container, ctrl } = setup();
    ctrl.value = "b";
    expect(ctrl.value).toBe("b");
    expect(container.querySelector(".custom-select-value").textContent).toBe("B");
    expect(container.querySelector('[data-value="b"]').classList.contains("selected")).toBe(true);
    expect(container.querySelector('[data-value="a"]').classList.contains("selected")).toBe(false);
  });

  it("设置无效值不改变当前状态", () => {
    const { ctrl } = setup("a");
    ctrl.value = "nonexistent";
    expect(ctrl.value).toBe("a");
  });

  it("onChange 回调在选项变更时触发", () => {
    const { container, ctrl, trigger } = setup();
    const values = [];
    ctrl.onChange = (v) => values.push(v);
    trigger.click();
    container.querySelector('[data-value="b"]').click();
    expect(values).toEqual(["b"]);
  });

  // ── 键盘导航 ──

  it("Enter 键展开/收起", () => {
    const { container, trigger } = setup();
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(container.classList.contains("open")).toBe(true);
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(container.classList.contains("open")).toBe(false);
  });

  it("Space 键展开/收起", () => {
    const { container, trigger } = setup();
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true }));
    expect(container.classList.contains("open")).toBe(true);
  });

  it("Escape 键关闭下拉", () => {
    const { container, trigger } = setup();
    trigger.click(); // 先打开
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(container.classList.contains("open")).toBe(false);
  });

  it("ArrowDown 选择下一项", () => {
    const { ctrl, trigger } = setup("a");
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(ctrl.value).toBe("b");
  });

  it("ArrowUp 选择上一项", () => {
    const { ctrl, trigger } = setup("b");
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(ctrl.value).toBe("a");
  });

  it("ArrowDown 在末尾不越界", () => {
    const { ctrl, trigger } = setup("c");
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(ctrl.value).toBe("c");
  });

  it("ArrowUp 在首位不越界", () => {
    const { ctrl, trigger } = setup("a");
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(ctrl.value).toBe("a");
  });

  it("外部点击关闭下拉", () => {
    const { container, trigger } = setup();
    trigger.click();
    expect(container.classList.contains("open")).toBe(true);
    document.body.click();
    expect(container.classList.contains("open")).toBe(false);
  });
});
