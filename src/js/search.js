/**
 * search.js — 搜索框模块
 * 提供防抖输入监听，隔离搜索逻辑。
 */

const DEBOUNCE_MS = 200;

let _input    = null;
let _callback = null;
let _timer    = null;

/**
 * 初始化搜索模块。
 * @param {HTMLInputElement} input          — 搜索输入框
 * @param {function(string): void} onSearch — 搜索回调（防抖后触发）
 */
export function init(input, onSearch) {
  _input    = input;
  _callback = onSearch;

  _input.addEventListener("input", _onInput);
}

/** 清空搜索框内容并触发一次空查询回调。 */
export function clear() {
  if (!_input) return;
  _input.value = "";
  _fireCallback("");
}

/** 将焦点移入搜索框。 */
export function focus() {
  if (_input) _input.focus();
}

// ── 内部 ──────────────────────────────────────────────────────────────────

function _onInput() {
  clearTimeout(_timer);
  _timer = setTimeout(() => {
    _fireCallback(_input.value.trim());
  }, DEBOUNCE_MS);
}

function _fireCallback(value) {
  if (_callback) _callback(value);
}
