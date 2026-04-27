/**
 * segment-tabs.js — 底部 All / Favorites segment + 实时计数
 */

import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";

let _root = null;
let _onChange = () => {};
let _mode = "all";
let _counts = { all: 0, favorites: 0 };

export function init(rootEl, onChange) {
  _root = rootEl;
  _onChange = onChange || (() => {});
  _indicator = null;
  _buttons = {};
  _mode = "all";
  render();
}

export function setMode(mode) {
  if (mode !== "all" && mode !== "favorites") return;
  if (mode === _mode) return;
  _mode = mode;
  render();
  telemetry.emit("clip-list:set-mode", { mode });
  _onChange(mode);
}

export function getMode() { return _mode; }

export function setCounts({ all, favorites }) {
  if (typeof all === "number") _counts.all = all;
  if (typeof favorites === "number") _counts.favorites = favorites;
  render();
}

export function refreshLabels() { render(); }

let _indicator = null;
let _buttons = {};

function render() {
  if (!_root) return;

  // 首次构建 DOM
  if (!_indicator) {
    _root.replaceChildren();
    const order = ["favorites", "all"];

    _indicator = document.createElement("span");
    _indicator.className = "segment-indicator";
    _root.appendChild(_indicator);

    for (const mode of order) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.setAttribute("role", "tab");
      btn.dataset.mode = mode;

      const label = document.createElement("span");
      label.className = "segment-tab-label";

      const count = document.createElement("span");
      count.className = "segment-tab-count";

      btn.append(label, count);
      btn.addEventListener("click", () => setMode(mode));
      _root.appendChild(btn);
      _buttons[mode] = { btn, label, count };
    }
  }

  // 增量更新状态
  _indicator.dataset.position = _mode === "favorites" ? "left" : "right";

  for (const mode of ["favorites", "all"]) {
    const { btn, label, count } = _buttons[mode];
    btn.className = "segment-tab" + (mode === _mode ? " active" : "");
    btn.setAttribute("aria-selected", String(mode === _mode));
    label.textContent = t(mode === "all" ? "tabs.all" : "tabs.favorites");
    count.textContent = String(mode === "all" ? _counts.all : _counts.favorites);
  }
}
