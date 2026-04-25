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

function render() {
  if (!_root) return;
  _root.replaceChildren();

  // 顺序：[Favorites] [All]，与左 ←/A → favorites、右 →/D → all 对应
  const order = ["favorites", "all"];

  // 滑动指示条
  const indicator = document.createElement("span");
  indicator.className = "segment-indicator";
  indicator.dataset.position = _mode === "favorites" ? "left" : "right";
  _root.appendChild(indicator);

  for (const mode of order) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "segment-tab" + (mode === _mode ? " active" : "");
    btn.setAttribute("role", "tab");
    btn.setAttribute("aria-selected", String(mode === _mode));
    btn.dataset.mode = mode;

    const label = document.createElement("span");
    label.className = "segment-tab-label";
    label.textContent = t(mode === "all" ? "tabs.all" : "tabs.favorites");

    const count = document.createElement("span");
    count.className = "segment-tab-count";
    count.textContent = String(mode === "all" ? _counts.all : _counts.favorites);

    btn.append(label, count);
    btn.addEventListener("click", () => setMode(mode));
    _root.appendChild(btn);
  }
}
