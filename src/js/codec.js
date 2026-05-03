/**
 * codec.js — 编解码工具面板
 *
 * 功能：手动快速编解码，支持 Base64 / URL / HTML / Unicode / Hex / ROT13 /
 *       MD5 / SHA / JSON / JWT / URL Parse / Timestamp / Number Base
 *
 * 架构：纯前端计算，仅剪贴板读写走 Tauri IPC
 */

import { setCodecVisible } from "./api.js";
import { t } from "../i18n/i18n.js";

// ── DOM refs ──
let _panelEl, _selectEl, _inputEl, _outputEl, _hintEl, _hintTextEl, _recentGroup;
let _visible = false;

// ── 最近使用（最多 5 项） ──
const RECENT_KEY = "clippy-codec-recent";
let _recentOps = [];

// ── 防抖 ──
const DEBOUNCE_MS = 150;
let _debounceTimer = null;

// ── 大文本保护阈值 ──
const AUTO_BAKE_LIMIT = 102400; // 100KB

// ── 操作反转映射 ──
const REVERSE_MAP = {
  "base64-encode": "base64-decode",
  "base64-decode": "base64-encode",
  "url-encode": "url-decode",
  "url-decode": "url-encode",
  "html-encode": "html-decode",
  "html-decode": "html-encode",
  "unicode-escape": "unicode-unescape",
  "unicode-unescape": "unicode-escape",
  "hex-encode": "hex-decode",
  "hex-decode": "hex-encode",
  "ts-to-date": "date-to-ts",
  "date-to-ts": "ts-to-date",
};

// ── 初始化 ──
export function init() {
  _panelEl    = document.getElementById("codec-panel");
  _selectEl   = document.getElementById("codec-select");
  _inputEl    = document.getElementById("codec-input");
  _outputEl   = document.getElementById("codec-output");
  _hintEl     = document.getElementById("codec-smart-hint");
  _hintTextEl = document.getElementById("codec-hint-text");
  _recentGroup = document.getElementById("codec-recent");

  // 加载最近使用
  try { _recentOps = JSON.parse(localStorage.getItem(RECENT_KEY) || "[]"); } catch { _recentOps = []; }
  _renderRecent();

  // 事件绑定
  _inputEl.addEventListener("input", _onInput);
  _selectEl.addEventListener("change", () => { _addRecent(_selectEl.value); _execute(); });

  document.getElementById("codec-swap-dir").addEventListener("click", _swapDirection);
  document.getElementById("codec-swap").addEventListener("click", _swapIO);
  document.getElementById("codec-clear").addEventListener("click", _clear);
  document.getElementById("codec-copy").addEventListener("click", _copyResult);
}

// ── 面板切换 ──
export async function toggle() {
  _visible = !_visible;
  _panelEl.classList.toggle("hidden", !_visible);
  if (_visible) {
    _inputEl.focus();
    _smartDetect();
  }
  try { await setCodecVisible(_visible); } catch (e) { console.warn("codec toggle:", e); }
}

export function isVisible() { return _visible; }

export async function hide() {
  if (!_visible) return;
  _visible = false;
  _panelEl.classList.add("hidden");
  try { await setCodecVisible(false); } catch {}
}

/** 外部填入内容（从剪贴板列表联动） */
export function setInput(text) {
  _inputEl.value = text;
  _smartDetect();
  _execute();
}

// ── 内部逻辑 ──

function _onInput() {
  clearTimeout(_debounceTimer);
  if (_inputEl.value.length > AUTO_BAKE_LIMIT) {
    _outputEl.textContent = t("codec.tooLarge") || "Content too large for auto-bake. Press Enter to execute.";
    return;
  }
  _debounceTimer = setTimeout(() => {
    _smartDetect();
    _execute();
  }, DEBOUNCE_MS);
}

/** Smart Detection：根据输入推荐操作 */
function _smartDetect() {
  const text = _inputEl.value.trim();
  if (!text) { _hintEl.hidden = true; return; }

  let suggested = null;
  if (/^eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/.test(text)) {
    suggested = "jwt-decode";
  } else if (/^\s*[\[{]/.test(text)) {
    try { JSON.parse(text); suggested = "json-format"; } catch {}
  } else if (/%[0-9A-Fa-f]{2}/.test(text) && !text.startsWith("http")) {
    suggested = "url-decode";
  } else if (/^[A-Za-z0-9+/=\n\r]+$/.test(text) && text.length > 8 && text.length % 4 === 0) {
    suggested = "base64-decode";
  } else if (/^\d{10}(\d{3})?$/.test(text)) {
    suggested = "ts-to-date";
  } else if (/^https?:\/\//.test(text)) {
    suggested = "url-parse";
  } else if (/^&[a-z]+;|&#\d+;|&#x[0-9a-f]+;/i.test(text)) {
    suggested = "html-decode";
  } else if (/\\u[0-9a-f]{4}/i.test(text)) {
    suggested = "unicode-unescape";
  }

  if (suggested && suggested !== _selectEl.value) {
    _hintTextEl.textContent = t("codec.suggest") || `Detected: try ${_getOpLabel(suggested)}`;
    _hintEl.hidden = false;
    _hintEl.onclick = () => { _selectEl.value = suggested; _addRecent(suggested); _execute(); _hintEl.hidden = true; };
  } else {
    _hintEl.hidden = true;
  }
}

function _getOpLabel(value) {
  const opt = _selectEl.querySelector(`option[value="${value}"]`);
  return opt ? opt.textContent : value;
}

/** 执行编解码操作 */
async function _execute() {
  const text = _inputEl.value;
  const op = _selectEl.value;
  if (!text || !op) { _outputEl.textContent = ""; return; }

  try {
    const result = await _runOp(op, text);
    _outputEl.textContent = result;
    _outputEl.classList.remove("codec-output--error");
  } catch (e) {
    _outputEl.textContent = `Error: ${e.message}`;
    _outputEl.classList.add("codec-output--error");
  }
}

/** 运行单个操作 */
async function _runOp(op, text) {
  switch (op) {
    // ── 编码/解码 ──
    case "base64-encode": return btoa(unescape(encodeURIComponent(text)));
    case "base64-decode": return decodeURIComponent(escape(atob(text.trim())));
    case "url-encode": return encodeURIComponent(text);
    case "url-decode": return decodeURIComponent(text);
    case "html-encode": return text.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
    case "html-decode": { const el = document.createElement("textarea"); el.innerHTML = text; return el.value; }
    case "unicode-escape": return [...text].map(c => { const code = c.codePointAt(0); return code > 127 ? `\\u${code.toString(16).padStart(4, "0")}` : c; }).join("");
    case "unicode-unescape": return text.replace(/\\u([0-9a-f]{4,6})/gi, (_, h) => String.fromCodePoint(parseInt(h, 16)));
    case "hex-encode": return [...new TextEncoder().encode(text)].map(b => b.toString(16).padStart(2, "0")).join(" ");
    case "hex-decode": return new TextDecoder().decode(new Uint8Array(text.trim().split(/[\s,]+/).map(h => parseInt(h, 16))));
    case "rot13": return text.replace(/[a-z]/gi, c => String.fromCharCode(c.charCodeAt(0) + (c.toLowerCase() < "n" ? 13 : -13)));

    // ── 哈希 ──
    case "md5": return _md5(text);
    case "sha1": return _hash("SHA-1", text);
    case "sha256": return _hash("SHA-256", text);
    case "sha512": return _hash("SHA-512", text);

    // ── 格式化 ──
    case "json-format": return JSON.stringify(JSON.parse(text), null, 2);
    case "json-minify": return JSON.stringify(JSON.parse(text));
    case "jwt-decode": return _jwtDecode(text);
    case "url-parse": return _urlParse(text);

    // ── 转换 ──
    case "ts-to-date": return _tsToDate(text);
    case "date-to-ts": return _dateToTs(text);
    case "num-base": return _numBase(text);

    default: return `Unknown operation: ${op}`;
  }
}

// ── 操作实现 ──

async function _hash(algo, text) {
  const buf = await crypto.subtle.digest(algo, new TextEncoder().encode(text));
  return [...new Uint8Array(buf)].map(b => b.toString(16).padStart(2, "0")).join("");
}

/** 最小 MD5 实现（Web Crypto 不支持 MD5） */
function _md5(str) {
  // 基于 Joseph Myers 的 MD5 实现（公有领域）
  function md5cycle(x, k) {
    let a = x[0], b = x[1], c = x[2], d = x[3];
    a = ff(a, b, c, d, k[0], 7, -680876936);  d = ff(d, a, b, c, k[1], 12, -389564586);
    c = ff(c, d, a, b, k[2], 17, 606105819);   b = ff(b, c, d, a, k[3], 22, -1044525330);
    a = ff(a, b, c, d, k[4], 7, -176418897);   d = ff(d, a, b, c, k[5], 12, 1200080426);
    c = ff(c, d, a, b, k[6], 17, -1473231341);  b = ff(b, c, d, a, k[7], 22, -45705983);
    a = ff(a, b, c, d, k[8], 7, 1770035416);    d = ff(d, a, b, c, k[9], 12, -1958414417);
    c = ff(c, d, a, b, k[10], 17, -42063);      b = ff(b, c, d, a, k[11], 22, -1990404162);
    a = ff(a, b, c, d, k[12], 7, 1804603682);   d = ff(d, a, b, c, k[13], 12, -40341101);
    c = ff(c, d, a, b, k[14], 17, -1502002290); b = ff(b, c, d, a, k[15], 22, 1236535329);
    a = gg(a, b, c, d, k[1], 5, -165796510);    d = gg(d, a, b, c, k[6], 9, -1069501632);
    c = gg(c, d, a, b, k[11], 14, 643717713);   b = gg(b, c, d, a, k[0], 20, -373897302);
    a = gg(a, b, c, d, k[5], 5, -701558691);    d = gg(d, a, b, c, k[10], 9, 38016083);
    c = gg(c, d, a, b, k[15], 14, -660478335);  b = gg(b, c, d, a, k[4], 20, -405537848);
    a = gg(a, b, c, d, k[9], 5, 568446438);     d = gg(d, a, b, c, k[14], 9, -1019803690);
    c = gg(c, d, a, b, k[3], 14, -187363961);   b = gg(b, c, d, a, k[8], 20, 1163531501);
    a = gg(a, b, c, d, k[13], 5, -1444681467);  d = gg(d, a, b, c, k[2], 9, -51403784);
    c = gg(c, d, a, b, k[7], 14, 1735328473);   b = gg(b, c, d, a, k[12], 20, -1926607734);
    a = hh(a, b, c, d, k[5], 4, -378558);       d = hh(d, a, b, c, k[8], 11, -2022574463);
    c = hh(c, d, a, b, k[11], 16, 1839030562);  b = hh(b, c, d, a, k[14], 23, -35309556);
    a = hh(a, b, c, d, k[1], 4, -1530992060);   d = hh(d, a, b, c, k[4], 11, 1272893353);
    c = hh(c, d, a, b, k[7], 16, -155497632);   b = hh(b, c, d, a, k[10], 23, -1094730640);
    a = hh(a, b, c, d, k[13], 4, 681279174);    d = hh(d, a, b, c, k[0], 11, -358537222);
    c = hh(c, d, a, b, k[3], 16, -722521979);   b = hh(b, c, d, a, k[6], 23, 76029189);
    a = hh(a, b, c, d, k[9], 4, -640364487);    d = hh(d, a, b, c, k[12], 11, -421815835);
    c = hh(c, d, a, b, k[15], 16, 530742520);   b = hh(b, c, d, a, k[2], 23, -995338651);
    a = ii(a, b, c, d, k[0], 6, -198630844);    d = ii(d, a, b, c, k[7], 10, 1126891415);
    c = ii(c, d, a, b, k[14], 15, -1416354905); b = ii(b, c, d, a, k[5], 21, -57434055);
    a = ii(a, b, c, d, k[12], 6, 1700485571);   d = ii(d, a, b, c, k[3], 10, -1894986606);
    c = ii(c, d, a, b, k[10], 15, -1051523);    b = ii(b, c, d, a, k[1], 21, -2054922799);
    a = ii(a, b, c, d, k[8], 6, 1873313359);    d = ii(d, a, b, c, k[15], 10, -30611744);
    c = ii(c, d, a, b, k[6], 15, -1560198380);  b = ii(b, c, d, a, k[13], 21, 1309151649);
    a = ii(a, b, c, d, k[4], 6, -145523070);    d = ii(d, a, b, c, k[11], 10, -1120210379);
    c = ii(c, d, a, b, k[2], 15, 718787259);    b = ii(b, c, d, a, k[9], 21, -343485551);
    x[0] = add32(a, x[0]); x[1] = add32(b, x[1]); x[2] = add32(c, x[2]); x[3] = add32(d, x[3]);
  }
  function cmn(q, a, b, x, s, t) { a = add32(add32(a, q), add32(x, t)); return add32((a << s) | (a >>> (32 - s)), b); }
  function ff(a, b, c, d, x, s, t) { return cmn((b & c) | (~b & d), a, b, x, s, t); }
  function gg(a, b, c, d, x, s, t) { return cmn((b & d) | (c & ~d), a, b, x, s, t); }
  function hh(a, b, c, d, x, s, t) { return cmn(b ^ c ^ d, a, b, x, s, t); }
  function ii(a, b, c, d, x, s, t) { return cmn(c ^ (b | ~d), a, b, x, s, t); }
  function add32(a, b) { return (a + b) & 0xFFFFFFFF; }

  const bytes = new TextEncoder().encode(str);
  const len = bytes.length;
  const tail = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  let state = [1732584193, -271733879, -1732584194, 271733878];
  let i;
  for (i = 64; i <= len; i += 64) {
    const block = [];
    for (let j = 0; j < 16; j++) {
      block[j] = bytes[i - 64 + j * 4] | (bytes[i - 64 + j * 4 + 1] << 8) | (bytes[i - 64 + j * 4 + 2] << 16) | (bytes[i - 64 + j * 4 + 3] << 24);
    }
    md5cycle(state, block);
  }
  for (let j = 0; j < 16; j++) tail[j] = 0;
  for (let j = i - 64; j < len; j++) {
    tail[(j - (i - 64)) >> 2] |= bytes[j] << (((j - (i - 64)) % 4) << 3);
  }
  tail[(len - (i - 64)) >> 2] |= 0x80 << (((len - (i - 64)) % 4) << 3);
  if ((len - (i - 64)) > 55) {
    md5cycle(state, tail);
    for (let j = 0; j < 16; j++) tail[j] = 0;
  }
  tail[14] = len * 8;
  md5cycle(state, tail);

  const hex = [];
  for (let j = 0; j < 4; j++) {
    for (let k = 0; k < 4; k++) {
      hex.push(((state[j] >> (k * 8)) & 0xFF).toString(16).padStart(2, "0"));
    }
  }
  return hex.join("");
}

function _jwtDecode(text) {
  const parts = text.trim().split(".");
  if (parts.length < 2) throw new Error("Invalid JWT format");
  const decode = (s) => {
    const padded = s.replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(decodeURIComponent(escape(atob(padded))));
  };
  const header = decode(parts[0]);
  const payload = decode(parts[1]);
  return [
    "=== Header ===",
    JSON.stringify(header, null, 2),
    "",
    "=== Payload ===",
    JSON.stringify(payload, null, 2),
  ].join("\n");
}

function _urlParse(text) {
  const url = new URL(text.trim());
  const lines = [
    `Protocol: ${url.protocol}`,
    `Host: ${url.host}`,
    `Pathname: ${url.pathname}`,
  ];
  if (url.port) lines.push(`Port: ${url.port}`);
  if (url.search) {
    lines.push("", "=== Query Parameters ===");
    for (const [k, v] of url.searchParams) {
      lines.push(`  ${k} = ${v}`);
    }
  }
  if (url.hash) lines.push(`Hash: ${url.hash}`);
  if (url.username) lines.push(`Username: ${url.username}`);
  return lines.join("\n");
}

function _tsToDate(text) {
  const num = parseInt(text.trim());
  if (isNaN(num)) throw new Error("Invalid timestamp");
  const ms = text.trim().length === 13 ? num : num * 1000;
  const d = new Date(ms);
  if (isNaN(d.getTime())) throw new Error("Invalid timestamp");
  return [
    `Local:  ${d.toLocaleString()}`,
    `UTC:    ${d.toUTCString()}`,
    `ISO:    ${d.toISOString()}`,
  ].join("\n");
}

function _dateToTs(text) {
  const d = new Date(text.trim());
  if (isNaN(d.getTime())) throw new Error("Invalid date string");
  return [
    `Seconds:      ${Math.floor(d.getTime() / 1000)}`,
    `Milliseconds: ${d.getTime()}`,
  ].join("\n");
}

function _numBase(text) {
  const trimmed = text.trim();
  let value;
  if (/^0x/i.test(trimmed)) value = parseInt(trimmed, 16);
  else if (/^0b/i.test(trimmed)) value = parseInt(trimmed.slice(2), 2);
  else if (/^0o/i.test(trimmed)) value = parseInt(trimmed.slice(2), 8);
  else value = parseInt(trimmed, 10);
  if (isNaN(value)) throw new Error("Invalid number");
  return [
    `Decimal: ${value}`,
    `Hex:     0x${value.toString(16).toUpperCase()}`,
    `Binary:  0b${value.toString(2)}`,
    `Octal:   0o${value.toString(8)}`,
  ].join("\n");
}

// ── UI helpers ──

function _swapDirection() {
  const current = _selectEl.value;
  const reverse = REVERSE_MAP[current];
  if (reverse) {
    _selectEl.value = reverse;
    _addRecent(reverse);
    _execute();
  }
}

function _swapIO() {
  const output = _outputEl.textContent;
  if (!output || _outputEl.classList.contains("codec-output--error")) return;
  _inputEl.value = output;
  _execute();
}

function _clear() {
  _inputEl.value = "";
  _outputEl.textContent = "";
  _outputEl.classList.remove("codec-output--error");
  _hintEl.hidden = true;
}

async function _copyResult() {
  const text = _outputEl.textContent;
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // fallback
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

function _addRecent(op) {
  _recentOps = [op, ..._recentOps.filter(o => o !== op)].slice(0, 5);
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(_recentOps)); } catch {}
  _renderRecent();
}

function _renderRecent() {
  if (!_recentGroup) return;
  _recentGroup.innerHTML = "";
  for (const op of _recentOps) {
    const label = _getOpLabel(op);
    if (!label) continue;
    const opt = document.createElement("option");
    opt.value = op;
    opt.textContent = label;
    _recentGroup.appendChild(opt);
  }
}

// ── 测试导出 ──
export const __test__ = {
  _runOp,
  _md5,
  _jwtDecode,
  _urlParse,
  _tsToDate,
  _dateToTs,
  _numBase,
  REVERSE_MAP,
};
