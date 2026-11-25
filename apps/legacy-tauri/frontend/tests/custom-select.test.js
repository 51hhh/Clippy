import { describe, it, expect, beforeEach, vi } from "vitest";
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

// ── 分组与动态选项（编解码面板的"最近使用"会在运行时重建 DOM） ──

/** 构造带分组的下拉框：一个动态分组 + 一个静态分组 */
function setupGrouped() {
  document.body.innerHTML = `
    <div class="custom-select" id="cs">
      <button class="custom-select-trigger" type="button">
        <span class="custom-select-value">A</span>
      </button>
      <ul class="custom-select-dropdown">
        <li class="custom-select-group" id="recent-group" hidden>
          <span class="custom-select-group-title">Recent</span>
          <ul class="custom-select-group-options" id="recent"></ul>
        </li>
        <li class="custom-select-group">
          <span class="custom-select-group-title">All</span>
          <ul class="custom-select-group-options">
            <li class="custom-select-option selected" data-value="a">A</li>
            <li class="custom-select-option" data-value="b">B</li>
          </ul>
        </li>
      </ul>
    </div>
  `;
  const container = document.getElementById("cs");
  const ctrl = initCustomSelect(container);
  return {
    container,
    ctrl,
    trigger: container.querySelector(".custom-select-trigger"),
    recent: document.getElementById("recent"),
  };
}

/** 模拟 codec.js 的 _renderRecent：重建"最近使用"里的选项 */
function renderRecent(recent, values) {
  recent.replaceChildren();
  for (const value of values) {
    const item = document.createElement("li");
    item.className = "custom-select-option";
    item.dataset.value = value;
    item.textContent = value.toUpperCase();
    recent.append(item);
  }
}

describe("custom-select 分组与动态选项", () => {
  beforeEach(() => { document.body.innerHTML = ""; });

  it("init 之后新增的选项依然可点击（事件委托）", () => {
    const { ctrl, recent, trigger } = setupGrouped();
    renderRecent(recent, ["b"]);
    trigger.click();
    recent.querySelector('[data-value="b"]').click();
    expect(ctrl.value).toBe("b");
  });

  it("动态选项参与键盘导航", () => {
    const { ctrl, recent, trigger } = setupGrouped();
    // 最近使用排在最前，选中项是静态分组里的 a，向上一步应落到动态项 x
    renderRecent(recent, ["x"]);
    trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(ctrl.value).toBe("x");
  });

  it("点击分组标题不改变取值", () => {
    const { container, ctrl } = setupGrouped();
    container.querySelector(".custom-select-group-title").click();
    expect(ctrl.value).toBe("a");
  });

  it("refresh 把选中态套到重建后的同值副本上", () => {
    const { container, ctrl, recent } = setupGrouped();
    renderRecent(recent, ["a"]);
    ctrl.refresh();
    expect(ctrl.value).toBe("a");
    // 文档顺序在前的副本承接选中态，静态分组里的旧副本必须让位
    expect(recent.querySelector('[data-value="a"]').classList.contains("selected")).toBe(true);
    expect(
      container.querySelectorAll(".custom-select-option.selected"),
    ).toHaveLength(1);
  });

  it("动态选项被清空后 refresh 不炸且保留当前值", () => {
    const { ctrl, recent } = setupGrouped();
    renderRecent(recent, ["a"]);
    ctrl.refresh();
    renderRecent(recent, []);
    ctrl.refresh();
    expect(ctrl.value).toBe("a");
  });
});

// 主窗口的 codec 侧栏在 window 上挂了键盘路由，Esc 是"关整个侧栏"。
// 下拉展开时 Esc 应该只收起下拉：内层消费掉的键不能再冒泡给外层状态机，
// 否则一次 Esc 同时收下拉 + 关侧栏，用户的输入内容跟着一起消失。
describe("Esc 的归属", () => {
  it("下拉展开时 Esc 收起下拉并阻止冒泡", () => {
    const { container, trigger } = setup();
    const bubbled = vi.fn();
    window.addEventListener("keydown", bubbled);

    trigger.click();
    expect(container.classList.contains("open")).toBe(true);

    trigger.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Escape", bubbles: true, cancelable: true,
    }));
    expect(container.classList.contains("open")).toBe(false);
    expect(bubbled).not.toHaveBeenCalled();

    // 下拉已经关着时不再拦，Esc 要能继续走到外层（关侧栏）
    trigger.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Escape", bubbles: true, cancelable: true,
    }));
    expect(bubbled).toHaveBeenCalledTimes(1);
    window.removeEventListener("keydown", bubbled);
  });
});
