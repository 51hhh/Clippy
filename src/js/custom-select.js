/**
 * custom-select.js — 纯 DOM 自定义下拉框组件
 *
 * 为什么不用原生 <select>：WebKitGTK 的原生下拉是独立 GTK 窗口，一弹出 webview 就失焦，
 * 无边框的悬浮主窗口会被自动隐藏，视觉上等同崩溃。
 *
 * DOM 约定：
 *   .custom-select                  容器（value 存在 dataset.value）
 *     .custom-select-trigger        触发按钮，承载键盘导航
 *     .custom-select-dropdown       下拉容器
 *       .custom-select-option       可选项（data-value）
 *       .custom-select-group        分组，标题 .custom-select-group-title 不可选
 *
 * 用法：
 *   import { initCustomSelect } from "./custom-select.js";
 *   const ctrl = initCustomSelect(containerEl);
 *   ctrl.value;         // 读取
 *   ctrl.value = "foo"; // 设置（不触发 onChange）
 *   ctrl.onChange = (v) => { ... }; // 监听变更
 *   ctrl.refresh();     // 外部重建过选项 DOM 后重新套用选中态
 */

export function initCustomSelect(container) {
  const trigger = container.querySelector(".custom-select-trigger");
  const dropdown = container.querySelector(".custom-select-dropdown");
  const valueSpan = container.querySelector(".custom-select-value");
  // 选项可能被外部动态重建（编解码面板的"最近使用"分组），因此每次实时查询而不是 init 时快照
  const options = () => [...container.querySelectorAll(".custom-select-option")];

  container.dataset.value =
    container.querySelector(".custom-select-option.selected")?.dataset.value || "";

  let _onChange = null;

  function applySelection(option) {
    options().forEach((other) => other.classList.remove("selected"));
    option.classList.add("selected");
    valueSpan.textContent = option.textContent;
    container.dataset.value = option.dataset.value || "";
  }

  function findOption(value) {
    if (!value) return null;
    return [...container.querySelectorAll(".custom-select-option")].find(
      (option) => option.dataset.value === value,
    ) || null;
  }

  function selectOption(option) {
    applySelection(option);
    container.classList.remove("open");
    trigger.focus();
    if (_onChange) _onChange(container.dataset.value);
  }

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    container.classList.toggle("open");
  });

  // 事件委托：动态插入的选项（最近使用）不必重新绑定
  dropdown.addEventListener("click", (e) => {
    const option = e.target?.closest?.(".custom-select-option");
    if (!option || !dropdown.contains(option)) return;
    e.stopPropagation();
    selectOption(option);
  });

  // 键盘导航
  trigger.addEventListener("keydown", (e) => {
    const isOpen = container.classList.contains("open");
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      container.classList.toggle("open");
    } else if (e.key === "Escape" && isOpen) {
      // 这一下 Esc 已经被下拉消费掉了，不能再冒泡给外层状态机：主窗口的键盘路由
      // 把 codec 侧栏里的 Esc 当成"关整个侧栏"，不拦就是一次 Esc 收下拉 + 关侧栏，
      // 输入框里的内容跟着一起没了。下拉本来就关着时不拦，Esc 照常走到外层。
      e.preventDefault();
      e.stopPropagation();
      container.classList.remove("open");
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const all = options();
      if (all.length === 0) return;
      const currentIdx = all.findIndex((option) => option.classList.contains("selected"));
      const next = e.key === "ArrowDown"
        ? Math.min(currentIdx + 1, all.length - 1)
        : Math.max(currentIdx - 1, 0);
      selectOption(all[next]);
    }
  });

  document.addEventListener("click", () => container.classList.remove("open"));

  return {
    get value() { return container.dataset.value; },
    set value(v) {
      const target = findOption(v);
      if (target) applySelection(target);
    },
    set onChange(fn) { _onChange = fn; },
    /** 选项 DOM 被外部重建后调用：把当前值重新标记为选中（例如"最近使用"里出现了同值副本） */
    refresh() {
      const target = findOption(container.dataset.value);
      if (target) applySelection(target);
    },
  };
}
