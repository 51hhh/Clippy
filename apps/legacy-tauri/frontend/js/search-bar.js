/**
 * search-bar.js — 顶部召唤式搜索条
 * 默认隐藏；外部调用 summon() 显示并聚焦；Esc 三段式由 app.js 调度。
 */

import { t } from "../i18n/i18n.js";
import * as telemetry from "./telemetry.js";

let _root = null;
let _input = null;
let _onQuery = () => {};
let _debounceTimer = null;
let _visible = false;

export function init(rootEl, onQuery) {
  _root = rootEl;
  _input = rootEl.querySelector(".search-bar-input");
  _onQuery = onQuery || (() => {});

  _input.addEventListener("input", () => {
    clearTimeout(_debounceTimer);
    const value = _input.value;
    _debounceTimer = setTimeout(() => _onQuery(value), 200);
  });
}

export function isVisible() { return _visible; }
export function getQuery() { return _input ? _input.value : ""; }
export function hasQuery() { return getQuery().trim().length > 0; }

export function summon(source = "keyboard") {
  if (!_root || _visible) return;
  _visible = true;
  _root.hidden = false;
  // 让浏览器有机会执行布局过渡
  requestAnimationFrame(() => _root.classList.add("visible"));
  _input.focus();
  telemetry.emit("search-bar:summon", { source });
}

/** Esc 三段：清空 → 收起 → 把"已隐藏"的指示返回 false 给 app.js */
export function dismissStage() {
  if (!_visible) return "panel";
  if (hasQuery()) {
    _input.value = "";
    _onQuery("");
    telemetry.emit("search-bar:dismiss", { stage: "clear" });
    return "clear";
  }
  hide();
  telemetry.emit("search-bar:dismiss", { stage: "hide" });
  return "hide";
}

export function hide() {
  if (!_root || !_visible) return;
  _visible = false;
  _root.classList.remove("visible");
  _root.hidden = true;
  if (_input) _input.value = "";
}

export function refreshLabels() {
  if (!_input) return;
  _input.placeholder = t("search.placeholder");
  const hint = _root.querySelector(".search-bar-hint");
  if (hint) hint.textContent = t("search.escHint");
}
