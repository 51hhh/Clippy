/**
 * custom-select.js — 纯 DOM 自定义下拉框组件
 *
 * 用法：
 *   import { initCustomSelect } from "./custom-select.js";
 *   const ctrl = initCustomSelect(containerEl);
 *   ctrl.value;         // 读取
 *   ctrl.value = "foo"; // 设置
 *   ctrl.onChange = (v) => { ... }; // 监听变更
 */

export function initCustomSelect(container) {
  const trigger = container.querySelector(".custom-select-trigger");
  const dropdown = container.querySelector(".custom-select-dropdown");
  const valueSpan = container.querySelector(".custom-select-value");
  const optionEls = [...container.querySelectorAll(".custom-select-option")];

  container.dataset.value = container.querySelector(".custom-select-option.selected")?.dataset.value || "";

  let _onChange = null;

  function selectOption(opt) {
    optionEls.forEach((o) => o.classList.remove("selected"));
    opt.classList.add("selected");
    valueSpan.textContent = opt.textContent;
    container.dataset.value = opt.dataset.value;
    container.classList.remove("open");
    trigger.focus();
    if (_onChange) _onChange(opt.dataset.value);
  }

  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    container.classList.toggle("open");
  });

  optionEls.forEach((opt) => {
    opt.addEventListener("click", (e) => {
      e.stopPropagation();
      selectOption(opt);
    });
  });

  // 键盘导航
  trigger.addEventListener("keydown", (e) => {
    const isOpen = container.classList.contains("open");
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      container.classList.toggle("open");
    } else if (e.key === "Escape" && isOpen) {
      container.classList.remove("open");
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const currentIdx = optionEls.findIndex((o) => o.classList.contains("selected"));
      const next = e.key === "ArrowDown"
        ? Math.min(currentIdx + 1, optionEls.length - 1)
        : Math.max(currentIdx - 1, 0);
      selectOption(optionEls[next]);
    }
  });

  document.addEventListener("click", () => container.classList.remove("open"));

  return {
    get value() { return container.dataset.value; },
    set value(v) {
      const target = container.querySelector(`.custom-select-option[data-value="${v}"]`);
      if (target) {
        optionEls.forEach((o) => o.classList.remove("selected"));
        target.classList.add("selected");
        valueSpan.textContent = target.textContent;
        container.dataset.value = v;
      }
    },
    set onChange(fn) { _onChange = fn; },
  };
}
