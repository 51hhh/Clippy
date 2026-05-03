/**
 * preview-panel.js — 富文本预览面板
 *
 * 渲染优先级（text 和 html 类型共享检测逻辑）：
 * 1. highlight.js 语言检测 → relevance > 5 → 代码高亮（badge: 语言名）
 * 2. Markdown 正则匹配 → marked 渲染（badge: MARKDOWN）
 * 3. html 类型 → DOMPurify 安全渲染（badge: RICH TEXT）
 * 4. 纯文本 → 原样显示（badge: TEXT）
 * 5. image → base64 PNG（badge: IMAGE）
 *
 * 安全：剪贴板原始数据不可修改，所有处理仅在预览渲染层面。
 * 性能：hljs/marked/DOMPurify 延迟加载，首次打开预览面板时才初始化。
 */

import { getClipImage, getClipDetail, setPreviewVisible, ocrAvailable, ocrImage, getConfig, fetchUrlMeta } from "./api.js";
import { t } from "../i18n/i18n.js";

// ── 延迟加载的库引用 ──
let hljs = null;
let DOMPurify = null;
let marked = null;
let _libsReady = false;
let _libsLoading = null;

/** 首次使用时加载 hljs/marked/DOMPurify 并完成初始化 */
async function ensureLibs() {
  if (_libsReady) return;
  if (_libsLoading) return _libsLoading;
  _libsLoading = _doLoadLibs();
  await _libsLoading;
  _libsReady = true;
  _libsLoading = null;
}

async function _doLoadLibs() {
  const [hljsMod, markedMod, purifyMod] = await Promise.all([
    import("highlight.js/lib/core"),
    import("marked"),
    import("dompurify"),
  ]);
  hljs = hljsMod.default;
  marked = markedMod.marked;
  DOMPurify = purifyMod.default;

  // ── highlight.js：按需注册常用语言 ──
  const langs = await Promise.all([
    import("highlight.js/lib/languages/javascript"),
    import("highlight.js/lib/languages/typescript"),
    import("highlight.js/lib/languages/python"),
    import("highlight.js/lib/languages/java"),
    import("highlight.js/lib/languages/cpp"),
    import("highlight.js/lib/languages/c"),
    import("highlight.js/lib/languages/csharp"),
    import("highlight.js/lib/languages/go"),
    import("highlight.js/lib/languages/rust"),
    import("highlight.js/lib/languages/bash"),
    import("highlight.js/lib/languages/sql"),
    import("highlight.js/lib/languages/json"),
    import("highlight.js/lib/languages/xml"),
    import("highlight.js/lib/languages/css"),
    import("highlight.js/lib/languages/yaml"),
    import("highlight.js/lib/languages/markdown"),
    import("highlight.js/lib/languages/ruby"),
    import("highlight.js/lib/languages/php"),
    import("highlight.js/lib/languages/swift"),
    import("highlight.js/lib/languages/kotlin"),
    import("highlight.js/lib/languages/lua"),
  ]);
  const langNames = [
    "javascript","typescript","python","java","cpp","c","csharp",
    "go","rust","bash","sql","json","xml","css","yaml","markdown",
    "ruby","php","swift","kotlin","lua",
  ];
  langNames.forEach((name, i) => hljs.registerLanguage(name, langs[i].default));

  // ── DOMPurify hook：清理背景色等主题冲突样式 ──
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.hasAttribute && node.hasAttribute("style")) {
      const cleaned = node.getAttribute("style").replace(STYLE_REMOVE_RE, "").trim();
      if (cleaned) {
        node.setAttribute("style", cleaned);
      } else {
        node.removeAttribute("style");
      }
    }
  });

  // ── Marked 配置 ──
  marked.setOptions({ breaks: true, gfm: true });
  const renderer = new marked.Renderer();
  renderer.code = function({ text, lang }) {
    let highlighted;
    if (lang && hljs.getLanguage(lang)) {
      highlighted = hljs.highlight(text, { language: lang }).value;
    } else {
      highlighted = hljs.highlightAuto(text).value;
    }
    return `<pre><code class="hljs">${DOMPurify.sanitize(highlighted)}</code></pre>`;
  };
  marked.use({ renderer });
}

// ── DOMPurify 配置（在 ensureLibs 中设置 hook） ──
const PURIFY_CONFIG = {
  ALLOWED_TAGS: [
    "h1","h2","h3","h4","h5","h6","p","br","hr","div","span",
    "a","b","i","u","em","strong","s","del","ins","sub","sup","small","mark",
    "ul","ol","li","dl","dt","dd",
    "table","thead","tbody","tfoot","tr","th","td","caption","colgroup","col",
    "blockquote","pre","code","kbd","var","samp",
    "img","video","audio","source","figure","figcaption",
    "details","summary","abbr","cite","q","time","ruby","rt","rp",
  ],
  ALLOWED_ATTR: [
    "href","src","alt","title","width","height","class","id","style",
    "target","rel","colspan","rowspan","headers","scope",
    "controls","autoplay","loop","muted","poster","preload",
    "type","start","reversed","value","datetime","lang","dir",
    "open","cite",
  ],
  ALLOW_DATA_ATTR: false,
  FORBID_TAGS: ["script","iframe","object","embed","form","input","textarea","button","select","option","link","meta","style"],
  FORBID_ATTR: ["onerror","onload","onclick","onmouseover","onfocus","onblur"],
};

const STYLE_REMOVE_RE = /\b(background-color|background(?!-)|color|position|z-index|opacity)\s*:[^;]*(;|$)/gi;

// URL 检测（仅匹配单行纯 URL 文本）
const URL_RE = /^https?:\/\/[^\s]+$/;
function isUrl(text) {
  return URL_RE.test(text) && text.length < 2048;
}

// JSON 检测：首字符为 { 或 [，尝试解析
function isJson(text) {
  const c = text[0];
  if (c !== "{" && c !== "[") return false;
  try { JSON.parse(text); return true; } catch { return false; }
}

// ── 编码 / 哈希 / 加密检测 ──────────────────────────────────

/** 可打印文本比例检测（防 Base64/Hex 误判） */
function isReadable(decoded) {
  if (!decoded || decoded.length === 0) return false;
  let printable = 0;
  const len = Math.min(decoded.length, 200);
  for (let i = 0; i < len; i++) {
    const c = decoded.charCodeAt(i);
    // ASCII 可打印 + 非 ASCII 正常 Unicode（排除控制字符 0x80-0x9F 和 BOM/FFFE）
    if ((c >= 32 && c <= 126) || (c >= 0xA0 && c !== 0xFFFE && c !== 0xFFFF) || c === 10 || c === 13 || c === 9) printable++;
  }
  return printable / len > 0.9;
}

/** Base64url 解码（JWT 用），正确处理 UTF-8 多字节字符 */
function base64urlDecode(str) {
  let b64 = str.replace(/-/g, "+").replace(/_/g, "/");
  while (b64.length % 4) b64 += "=";
  const binary = atob(b64);
  const bytes = Uint8Array.from(binary, c => c.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

/** JWT 检测：eyJ 开头 + 恰好 2 个 '.' + 每段合法 Base64url */
const BASE64URL_PART_RE = /^[A-Za-z0-9_-]+={0,2}$/;
function isJwt(text) {
  if (!text.startsWith("eyJ")) return false;
  const parts = text.split(".");
  if (parts.length !== 3) return false;
  return parts.every(p => BASE64URL_PART_RE.test(p));
}

/** 解析 JWT → { header, payload, signature } */
function parseJwt(text) {
  const [h, p, s] = text.split(".");
  let header, payload;
  try { header = JSON.parse(base64urlDecode(h)); } catch { header = null; }
  try { payload = JSON.parse(base64urlDecode(p)); } catch { payload = null; }
  return { header, payload, signature: s };
}

/** Base64 检测 */
const BASE64_RE = /^[A-Za-z0-9+/]{24,}={0,2}$/;
function isBase64(text) {
  // 支持多行 Base64（PEM 证书体等）：先去除空白再检测
  const clean = text.replace(/[\s\r\n]/g, "");
  if (clean.length < 24 || clean.length % 4 !== 0) return false;
  if (!BASE64_RE.test(clean)) return false;
  try {
    const decoded = atob(clean);
    // 检查是否为图片魔数
    if (decoded.startsWith("\x89PNG") || decoded.startsWith("GIF8") ||
        decoded.startsWith("\xFF\xD8\xFF") || decoded.startsWith("RIFF")) {
      return { type: "base64-image", decoded, original: text };
    }
    if (isReadable(decoded)) {
      return { type: "base64", decoded, original: text };
    }
    return false;
  } catch { return false; }
}

/** URL 编码检测 */
const URL_ENCODED_RE = /%[0-9A-Fa-f]{2}/g;
function isUrlEncoded(text) {
  if (text.includes("\n")) return false;
  const matches = text.match(URL_ENCODED_RE);
  if (!matches || matches.length < 2) return false;
  try {
    const decoded = decodeURIComponent(text);
    if (decoded === text) return false;
    return { type: "url-encoded", decoded, original: text };
  } catch { return false; }
}

/** HTML 实体检测 */
const HTML_ENTITY_RE = /&(?:#\d+|#x[0-9a-f]+|\w+);/gi;
function isHtmlEntities(text) {
  const matches = text.match(HTML_ENTITY_RE);
  if (!matches || matches.length < 2) return false;
  const el = document.createElement("textarea");
  el.innerHTML = text;
  const decoded = el.value;
  if (decoded === text) return false;
  return { type: "html-entity", decoded, original: text };
}

/** Unicode 转义检测 */
const UNICODE_RE = /\\u[0-9a-fA-F]{4}/g;
function isUnicodeEscape(text) {
  const matches = text.match(UNICODE_RE);
  if (!matches || matches.length < 2) return false;
  try {
    const decoded = text.replace(UNICODE_RE, (m) =>
      String.fromCharCode(parseInt(m.slice(2), 16))
    );
    if (decoded === text) return false;
    return { type: "unicode", decoded, original: text };
  } catch { return false; }
}

/** Hex 编码检测 */
const HEX_RE = /^(?:0x)?[0-9a-f]+$/i;
function isHexEncoded(text) {
  const clean = text.startsWith("0x") || text.startsWith("0X") ? text.slice(2) : text;
  if (clean.length < 8 || clean.length % 2 !== 0 || clean.length > 2000) return false;
  if (!HEX_RE.test(text)) return false;
  // 排除哈希（精确长度匹配在 identifyHash 中处理）
  if ([32, 40, 64, 128].includes(clean.length)) return false;
  try {
    const chars = [];
    for (let i = 0; i < clean.length; i += 2) {
      chars.push(String.fromCharCode(parseInt(clean.slice(i, i + 2), 16)));
    }
    const decoded = chars.join("");
    if (isReadable(decoded)) {
      return { type: "hex", decoded, original: text };
    }
    return false;
  } catch { return false; }
}

/** 统一编码检测入口 */
function detectEncoding(text) {
  // 顺序：URL编码 → HTML实体 → Unicode转义 → Base64 → Hex
  return isUrlEncoded(text) || isHtmlEntities(text) || isUnicodeEscape(text)
    || isBase64(text) || isHexEncoded(text) || false;
}

/** 哈希类型识别 */
const HASH_PATTERNS = [
  { re: /^[0-9a-f]{32}$/i,  name: "MD5" },
  { re: /^[0-9a-f]{40}$/i,  name: "SHA-1" },
  { re: /^[0-9a-f]{64}$/i,  name: "SHA-256" },
  { re: /^[0-9a-f]{128}$/i, name: "SHA-512" },
  { re: /^\$2[aby]\$\d{2}\$.{53}$/,       name: "bcrypt" },
  { re: /^\$argon2(id?|d)\$v=\d+\$m=/,    name: "Argon2" },
];
function identifyHash(text) {
  for (const { re, name } of HASH_PATTERNS) {
    if (re.test(text)) return name;
  }
  return null;
}

/** 加密内容检测 */
const OPENSSL_PREFIX = "U2FsdGVkX1"; // Base64 of "Salted__"
function detectEncrypted(text) {
  if (text.startsWith("-----BEGIN PGP MESSAGE-----")) return "PGP";
  if (text.startsWith("-----BEGIN ENCRYPTED PRIVATE KEY-----")) return "ENCRYPTED-KEY";
  if (text.startsWith(OPENSSL_PREFIX)) return "AES-OpenSSL";
  // 通用加密检测：长 Base64 + 解码后不可读 + 高随机性
  const clean = text.replace(/[\s\r\n]/g, "");
  if (clean.length >= 64 && clean.length % 4 === 0 && BASE64_RE.test(clean)) {
    try {
      const decoded = atob(clean);
      if (!isReadable(decoded) && decoded.length >= 48 && decoded.length % 16 === 0) {
        return "AES-Generic";
      }
    } catch { /* not base64 */ }
  }
  return null;
}

// ── 颜色值 / 时间戳 / UUID / IP 检测 ───────────────────────

/** 颜色值检测：#hex / rgb() / hsl() / oklch() */
const COLOR_HEX_RE = /^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const COLOR_FUNC_RE = /^(rgba?|hsla?|oklch|oklab|hwb)\(\s*[\d.,/%\s-]+\)$/i;
function isColor(text) {
  return COLOR_HEX_RE.test(text) || COLOR_FUNC_RE.test(text);
}

/** 解析颜色为标准化 CSS 值（用于渲染色块） */
function normalizeColor(text) {
  // 浏览器可直接渲染这些格式，返回原文即可
  return text;
}

/** Unix 时间戳检测（10位秒 / 13位毫秒） */
const TIMESTAMP_RE = /^\d{10,13}$/;
function isTimestamp(text) {
  if (!TIMESTAMP_RE.test(text)) return false;
  const n = parseInt(text, 10);
  // 合理范围：2000-01-01 到 2100-01-01
  const secs = text.length === 13 ? n / 1000 : n;
  return secs >= 946684800 && secs <= 4102444800;
}

/** 时间戳转日期展示 */
function formatTimestamp(text) {
  const n = parseInt(text, 10);
  const ms = text.length === 13 ? n : n * 1000;
  const d = new Date(ms);
  return {
    utc: d.toISOString(),
    local: d.toLocaleString(),
    relative: _relativeTime(d),
    precision: text.length === 13 ? "ms" : "s",
  };
}

function _relativeTime(date) {
  const diff = Date.now() - date.getTime();
  const absDiff = Math.abs(diff);
  const future = diff < 0;
  const prefix = future ? "in " : "";
  const suffix = future ? "" : " ago";
  if (absDiff < 60000) return "just now";
  if (absDiff < 3600000) return `${prefix}${Math.floor(absDiff / 60000)} min${suffix}`;
  if (absDiff < 86400000) return `${prefix}${Math.floor(absDiff / 3600000)} hr${suffix}`;
  return `${prefix}${Math.floor(absDiff / 86400000)} days${suffix}`;
}

/** UUID 检测 + 版本识别 */
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-([0-9a-f])[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
function isUuid(text) {
  return UUID_RE.test(text);
}

function uuidVersion(text) {
  const m = text.match(UUID_RE);
  if (!m) return null;
  const v = parseInt(m[1], 16);
  if (v >= 1 && v <= 5) return `v${v}`;
  if (v === 7) return "v7";
  return null;
}

/** IP 地址检测 */
const IPV4_RE = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(\/\d{1,2})?$/;
const IPV6_RE = /^([0-9a-f:]+)(\/\d{1,3})?$/i;
function isIpAddress(text) {
  const m4 = text.match(IPV4_RE);
  if (m4) {
    return [m4[1], m4[2], m4[3], m4[4]].every(o => parseInt(o) <= 255);
  }
  if (!IPV6_RE.test(text)) return false;
  const addr = text.split("/")[0];
  // :: 只能出现一次
  const doubleColonCount = (addr.match(/::/g) || []).length;
  if (doubleColonCount > 1) return false;
  const parts = addr.split(":");
  if (parts.length < 3 || parts.length > 8) return false;
  if (doubleColonCount === 0 && parts.length !== 8) return false;
  return parts.every(p => p === "" || /^[0-9a-f]{1,4}$/i.test(p));
}

function ipInfo(text) {
  if (IPV4_RE.test(text)) {
    const addr = text.split("/")[0];
    const parts = addr.split(".").map(Number);
    let type = "Public";
    if (parts[0] === 10) type = "Private (Class A)";
    else if (parts[0] === 172 && parts[1] >= 16 && parts[1] <= 31) type = "Private (Class B)";
    else if (parts[0] === 192 && parts[1] === 168) type = "Private (Class C)";
    else if (parts[0] === 127) type = "Loopback";
    else if (parts[0] === 169 && parts[1] === 254) type = "Link-local";
    return { version: "IPv4", type, cidr: text.includes("/") };
  }
  return { version: "IPv6", type: text.startsWith("fe80") ? "Link-local" : text.startsWith("fc") || text.startsWith("fd") ? "ULA" : "Global", cidr: text.includes("/") };
}

// ── Email / MAC / Cron / 日期字符串检测 ─────────────────────

/** Email 地址检测（严格：单行、无空格、有@和域名） */
const EMAIL_RE = /^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+$/;
function isEmail(text) {
  return text.length >= 5 && text.length <= 254 && !text.includes("\n") && EMAIL_RE.test(text);
}

function emailInfo(text) {
  const [local, domain] = text.split("@");
  return { local, domain };
}

/** MAC 地址检测（EUI-48: XX:XX:XX:XX:XX:XX 或 XX-XX-XX-XX-XX-XX，EUI-64 也支持） */
const MAC48_RE = /^([0-9a-f]{2}[:\-]){5}[0-9a-f]{2}$/i;
const MAC64_RE = /^([0-9a-f]{2}[:\-]){7}[0-9a-f]{2}$/i;
const MAC_CISCO_RE = /^[0-9a-f]{4}\.[0-9a-f]{4}\.[0-9a-f]{4}$/i;
function isMacAddress(text) {
  return MAC48_RE.test(text) || MAC64_RE.test(text) || MAC_CISCO_RE.test(text);
}

function macInfo(text) {
  let format = "EUI-48";
  if (MAC64_RE.test(text)) format = "EUI-64";
  else if (MAC_CISCO_RE.test(text)) format = "Cisco";
  // 规范化为 : 分隔
  const normalized = text.replace(/[-\.]/g, ":").toUpperCase();
  // OUI = 前 3 字节
  const octets = normalized.split(":");
  const oui = octets.slice(0, 3).join(":");
  // 检测是否为本地管理/组播
  const firstByte = parseInt(octets[0], 16);
  const multicast = !!(firstByte & 0x01);
  const localAdmin = !!(firstByte & 0x02);
  return { format, normalized, oui, multicast, localAdmin };
}

/** Cron 表达式检测（5 或 6 字段） */
const CRON_FIELD_RE = /^(\*|([0-9]+(-[0-9]+)?)(\/[0-9]+)?|(\*\/[0-9]+)|([0-9,]+(-[0-9]+)*))(\/[0-9]+)?$/;
function isCron(text) {
  const parts = text.trim().split(/\s+/);
  if (parts.length < 5 || parts.length > 6) return false;
  return parts.every(p => CRON_FIELD_RE.test(p));
}

function cronDescribe(text) {
  const parts = text.trim().split(/\s+/);
  const hasSec = parts.length === 6;
  const [min, hour, dom, mon, dow] = hasSec ? parts.slice(1) : parts;

  const descParts = [];
  if (hasSec) descParts.push(_cronField(parts[0], "second"));
  descParts.push(_cronField(min, "minute"));
  descParts.push(_cronField(hour, "hour"));
  descParts.push(_cronField(dom, "day"));
  descParts.push(_cronField(mon, "month"));
  descParts.push(_cronField(dow, "weekday"));

  return { fields: parts.length, description: descParts.filter(Boolean).join(", ") || "Every minute" };
}

const _CRON_WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
function _cronField(val, unit) {
  if (val === "*") return null; // every
  if (val.startsWith("*/")) return `every ${val.slice(2)} ${unit}s`;
  if (val.includes(",")) return `${unit} ${val}`;
  if (val.includes("-")) return `${unit} ${val}`;
  if (unit === "weekday" && /^\d$/.test(val)) return _CRON_WEEKDAYS[+val] || val;
  return `${unit} ${val}`;
}

/** 日期字符串检测（ISO 8601 / RFC 2822 / 常见格式） */
const ISO_DATE_RE = /^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}(:\d{2})?(\.\d+)?(Z|[+-]\d{2}:?\d{2})?)?$/;
const RFC_DATE_RE = /^(Mon|Tue|Wed|Thu|Fri|Sat|Sun),?\s+\d{1,2}\s+(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{4}/;
const COMMON_DATE_RE = /^\d{4}[/\-.]\d{2}[/\-.]\d{2}(\s+\d{2}:\d{2}(:\d{2})?)?$/;
function isDateString(text) {
  if (text.length < 8 || text.length > 40) return false;
  if (ISO_DATE_RE.test(text) || RFC_DATE_RE.test(text) || COMMON_DATE_RE.test(text)) {
    const d = new Date(text);
    return !isNaN(d.getTime());
  }
  return false;
}

function dateInfo(text) {
  const d = new Date(text);
  const now = new Date();
  const diff = d - now;
  const absDiff = Math.abs(diff);
  let relative;
  if (absDiff < 60_000) relative = "just now";
  else if (absDiff < 3_600_000) relative = `${Math.round(absDiff / 60_000)} min ${diff > 0 ? "from now" : "ago"}`;
  else if (absDiff < 86_400_000) relative = `${Math.round(absDiff / 3_600_000)} hr ${diff > 0 ? "from now" : "ago"}`;
  else relative = `${Math.round(absDiff / 86_400_000)} days ${diff > 0 ? "from now" : "ago"}`;

  return {
    local: d.toLocaleString(),
    utc: d.toUTCString(),
    iso: d.toISOString(),
    timestamp: Math.floor(d.getTime() / 1000),
    relative,
  };
}

// ── 语义版本 / 数字进制 / CSS 渐变 / 数据大小检测 ─────────

/** 语义版本号检测 (SemVer) */
const SEMVER_RE = /^v?(\d+)\.(\d+)\.(\d+)(?:-([a-zA-Z0-9]+(?:\.[a-zA-Z0-9]+)*))?(?:\+([a-zA-Z0-9]+(?:\.[a-zA-Z0-9]+)*))?$/;
function isSemver(text) {
  return SEMVER_RE.test(text.trim());
}

function semverInfo(text) {
  const m = text.trim().match(SEMVER_RE);
  if (!m) return null;
  return {
    major: parseInt(m[1]),
    minor: parseInt(m[2]),
    patch: parseInt(m[3]),
    preRelease: m[4] || null,
    build: m[5] || null,
    normalized: `${m[1]}.${m[2]}.${m[3]}${m[4] ? `-${m[4]}` : ""}${m[5] ? `+${m[5]}` : ""}`,
  };
}

/** 数字进制检测：0x (hex), 0b (binary), 0o (octal), 或纯大整数 */
const HEX_NUM_RE = /^0x[0-9a-f]+$/i;
const BIN_NUM_RE = /^0b[01]+$/i;
const OCT_NUM_RE = /^0o[0-7]+$/i;
function isNumberBase(text) {
  return HEX_NUM_RE.test(text) || BIN_NUM_RE.test(text) || OCT_NUM_RE.test(text);
}

function numberBaseInfo(text) {
  let base, value;
  if (HEX_NUM_RE.test(text)) {
    base = 16;
    value = parseInt(text, 16);
  } else if (BIN_NUM_RE.test(text)) {
    base = 2;
    value = parseInt(text.slice(2), 2);
  } else {
    base = 8;
    value = parseInt(text.slice(2), 8);
  }
  return {
    base,
    decimal: value,
    hex: `0x${value.toString(16).toUpperCase()}`,
    binary: `0b${value.toString(2)}`,
    octal: `0o${value.toString(8)}`,
  };
}

/** CSS 渐变检测 */
const GRADIENT_RE = /^(linear-gradient|radial-gradient|conic-gradient|repeating-linear-gradient|repeating-radial-gradient|repeating-conic-gradient)\([\s\S]+\)$/i;
function isGradient(text) {
  return GRADIENT_RE.test(text.trim());
}

/** 数据大小检测与转换 */
const DATA_SIZE_RE = /^(\d+(?:\.\d+)?)\s*(B|KB|KiB|MB|MiB|GB|GiB|TB|TiB|PB|PiB|bytes?)$/i;
function isDataSize(text) {
  return DATA_SIZE_RE.test(text.trim());
}

const _SIZE_TO_BYTES = {
  b: 1, byte: 1, bytes: 1,
  kb: 1e3, kib: 1024,
  mb: 1e6, mib: 1048576,
  gb: 1e9, gib: 1073741824,
  tb: 1e12, tib: 1099511627776,
  pb: 1e15, pib: 1125899906842624,
};

function dataSizeInfo(text) {
  const m = text.trim().match(DATA_SIZE_RE);
  if (!m) return null;
  const value = parseFloat(m[1]);
  const unit = m[2].toLowerCase();
  const bytes = value * (_SIZE_TO_BYTES[unit] || 1);
  const conversions = [];
  const units = [
    ["B", 1], ["KB", 1e3], ["KiB", 1024],
    ["MB", 1e6], ["MiB", 1048576],
    ["GB", 1e9], ["GiB", 1073741824],
    ["TB", 1e12], ["TiB", 1099511627776],
  ];
  for (const [u, factor] of units) {
    const v = bytes / factor;
    if (v >= 0.01 && v < 1e6) {
      conversions.push(`${v % 1 === 0 ? v : v.toFixed(2)} ${u}`);
    }
  }
  return { bytes, conversions };
}

// ── 正则 / 坐标 / MIME / 数学表达式 / HTTP 状态码 ─────────

/** 正则表达式检测 /pattern/flags */
const REGEX_RE = /^\/(.+)\/([gimsuy]{0,6})$/s;
function isRegex(text) {
  if (!text.startsWith("/") || text.length < 3) return false;
  const m = text.match(REGEX_RE);
  if (!m) return false;
  try { new RegExp(m[1], m[2]); return true; } catch { return false; }
}

function regexInfo(text) {
  const m = text.match(REGEX_RE);
  const flags = m[2] || "";
  const flagDescs = [];
  if (flags.includes("g")) flagDescs.push("global");
  if (flags.includes("i")) flagDescs.push("case-insensitive");
  if (flags.includes("m")) flagDescs.push("multiline");
  if (flags.includes("s")) flagDescs.push("dotAll");
  if (flags.includes("u")) flagDescs.push("unicode");
  if (flags.includes("y")) flagDescs.push("sticky");
  return { pattern: m[1], flags, flagDescs };
}

/** 坐标检测（lat, lng）*/
const COORD_RE = /^(-?\d{1,3}(?:\.\d+)?)\s*[,\s]\s*(-?\d{1,3}(?:\.\d+)?)$/;
function isCoordinate(text) {
  const m = text.match(COORD_RE);
  if (!m) return false;
  const lat = parseFloat(m[1]);
  const lng = parseFloat(m[2]);
  return lat >= -90 && lat <= 90 && lng >= -180 && lng <= 180;
}

function coordInfo(text) {
  const m = text.match(COORD_RE);
  const lat = parseFloat(m[1]);
  const lng = parseFloat(m[2]);
  const latDir = lat >= 0 ? "N" : "S";
  const lngDir = lng >= 0 ? "E" : "W";
  // DMS conversion
  const toDms = (deg) => {
    const abs = Math.abs(deg);
    const d = Math.floor(abs);
    const min = Math.floor((abs - d) * 60);
    const sec = ((abs - d) * 60 - min) * 60;
    return `${d}°${min}′${sec.toFixed(1)}″`;
  };
  return {
    lat, lng,
    dms: `${toDms(lat)}${latDir}, ${toDms(lng)}${lngDir}`,
    decimal: `${lat.toFixed(6)}, ${lng.toFixed(6)}`,
  };
}

/** MIME type 检测 */
const MIME_RE = /^(application|audio|font|image|message|model|multipart|text|video|chemical)\/[a-z0-9][a-z0-9!#$&\-.^_+]+$/i;
function isMimeType(text) {
  return text.length >= 3 && text.length <= 100 && MIME_RE.test(text);
}

const _MIME_DESCS = {
  "application/json": "JSON data",
  "application/xml": "XML document",
  "application/pdf": "PDF document",
  "application/zip": "ZIP archive",
  "application/gzip": "Gzip archive",
  "application/javascript": "JavaScript",
  "application/typescript": "TypeScript",
  "application/octet-stream": "Binary data",
  "application/x-tar": "TAR archive",
  "application/wasm": "WebAssembly",
  "text/html": "HTML document",
  "text/css": "CSS stylesheet",
  "text/plain": "Plain text",
  "text/markdown": "Markdown",
  "text/csv": "CSV data",
  "image/png": "PNG image",
  "image/jpeg": "JPEG image",
  "image/gif": "GIF image",
  "image/svg+xml": "SVG image",
  "image/webp": "WebP image",
  "audio/mpeg": "MP3 audio",
  "audio/ogg": "Ogg audio",
  "video/mp4": "MP4 video",
  "video/webm": "WebM video",
};

function mimeInfo(text) {
  const lower = text.toLowerCase();
  const [type, subtype] = lower.split("/");
  return {
    type,
    subtype,
    description: _MIME_DESCS[lower] || `${type} content`,
  };
}

/** 数学表达式检测（简单四则运算 + 幂 + 括号） */
const MATH_EXPR_RE = /^[\d\s+\-*/().^%]+$/;
function isMathExpr(text) {
  if (text.length < 3 || text.length > 100) return false;
  if (!MATH_EXPR_RE.test(text)) return false;
  // 必须有操作符
  if (!/[+\-*/^%]/.test(text)) return false;
  // 必须有数字
  if (!/\d/.test(text)) return false;
  // 不能是纯数字+小数点（避免和版本号冲突）
  if (/^[\d.]+$/.test(text.trim())) return false;
  try {
    // 安全求值：替换 ^ 为 **，只允许数字和运算符
    const safe = text.replace(/\^/g, "**");
    if (/[a-zA-Z_$]/.test(safe)) return false;
    const result = Function(`"use strict"; return (${safe})`)();
    return typeof result === "number" && isFinite(result);
  } catch { return false; }
}

function mathEval(text) {
  const safe = text.replace(/\^/g, "**");
  const result = Function(`"use strict"; return (${safe})`)();
  return { expression: text, result };
}

/** HTTP 状态码检测 */
const HTTP_STATUS_RE = /^[1-5]\d{2}$/;
const _HTTP_CODES = {
  100: "Continue", 101: "Switching Protocols",
  200: "OK", 201: "Created", 202: "Accepted", 204: "No Content",
  301: "Moved Permanently", 302: "Found", 304: "Not Modified", 307: "Temporary Redirect", 308: "Permanent Redirect",
  400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found", 405: "Method Not Allowed",
  408: "Request Timeout", 409: "Conflict", 410: "Gone", 413: "Payload Too Large", 415: "Unsupported Media Type",
  418: "I'm a teapot", 422: "Unprocessable Entity", 429: "Too Many Requests",
  500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable", 504: "Gateway Timeout",
};
function isHttpStatus(text) {
  if (!HTTP_STATUS_RE.test(text.trim())) return false;
  return !!_HTTP_CODES[parseInt(text.trim())];
}

function httpStatusInfo(code) {
  const num = parseInt(code.trim());
  const cat = num < 200 ? "Informational" : num < 300 ? "Success" : num < 400 ? "Redirection" : num < 500 ? "Client Error" : "Server Error";
  return { code: num, message: _HTTP_CODES[num], category: cat };
}

// Markdown 特征正则（需要多个特征匹配才认定为 Markdown）
const MD_HEADING = /^#{1,6}\s+\S/m;
const MD_FENCED_CODE = /^```/m;
const MD_LIST = /^\s{0,3}[-*+]\s+\S/m;
const MD_ORDERED_LIST = /^\s{0,3}\d+\.\s+\S/m;
const MD_BLOCKQUOTE = /^>\s+\S/m;
const MD_LINK = /\[.+?\]\(.+?\)/;
const MD_IMAGE = /!\[.*?\]\(.+?\)/;
const MD_BOLD = /\*\*.+?\*\*/;
const MD_HR = /^---$/m;

function isMarkdown(text) {
  let score = 0;
  if (MD_HEADING.test(text)) score += 2;
  if (MD_FENCED_CODE.test(text)) score += 2;
  if (MD_LIST.test(text)) score++;
  if (MD_ORDERED_LIST.test(text)) score++;
  if (MD_BLOCKQUOTE.test(text)) score++;
  if (MD_LINK.test(text)) score++;
  if (MD_IMAGE.test(text)) score += 2;
  if (MD_BOLD.test(text)) score++;
  if (MD_HR.test(text)) score++;
  return score >= 2;
}

// 代码检测阈值
const CODE_RELEVANCE_THRESHOLD = 5;
const HLJS_CACHE_MAX = 200;
const _hljsCache = new Map(); // content_hash → { language, relevance, value }

let _panelEl;
let _contentEl;
let _badgeEl;
let _metaEl;
let _visible = false;
let _currentClipId = null;

export function init() {
  _panelEl   = document.getElementById("preview-panel");
  _contentEl = document.getElementById("preview-content");
  _badgeEl   = document.getElementById("preview-type-badge");
  _metaEl    = document.getElementById("preview-meta");

  // 拦截预览面板内的链接点击，防止 webview 导航丢失
  _contentEl.addEventListener("click", (e) => {
    const a = e.target.closest("a[href]");
    if (a) {
      e.preventDefault();
      e.stopPropagation();
    }
  });
}

export async function toggle() {
  _visible = !_visible;
  _panelEl.classList.toggle("hidden", !_visible);
  if (_visible) _currentClipId = null;
  try {
    await setPreviewVisible(_visible);
  } catch (e) {
    console.warn("切换预览面板失败:", e);
  }
}

/** 清空预览内容，释放内存（窗口隐藏时调用） */
export function clearContent() {
  _contentEl.innerHTML = "";
  _contentEl.className = "preview-content";
  _badgeEl.textContent = "";
  _metaEl.textContent = "";
  _currentClipId = null;
}

export function isVisible() {
  return _visible;
}

export async function hide() {
  if (!_visible) return;
  _visible = false;
  _panelEl.classList.add("hidden");
  try {
    await setPreviewVisible(false);
  } catch (e) {
    console.warn("隐藏预览面板失败:", e);
  }
}

let _debounceTimer = null;
const DEBOUNCE_MS = 80;

export function updatePreview(clip) {
  clearTimeout(_debounceTimer);
  if (!_visible || !clip) {
    _doUpdatePreview(clip);
    return;
  }
  _debounceTimer = setTimeout(() => _doUpdatePreview(clip), DEBOUNCE_MS);
}

async function _doUpdatePreview(clip) {
  if (!_visible || !clip) {
    _contentEl.innerHTML = "";
    _contentEl.className = "preview-content";
    _badgeEl.textContent = "";
    _metaEl.textContent = "";
    _currentClipId = null;
    return;
  }

  if (clip.id === _currentClipId) return;
  _currentClipId = clip.id;

  const size = clip.byte_size;
  _metaEl.textContent = size > 1024
    ? `${(size / 1024).toFixed(1)} KB`
    : `${size} B`;

  _contentEl.innerHTML = "";
  _contentEl.className = "preview-content";

  if (clip.content_type === "image") {
    await renderImage(clip);
    return;
  }

  // text 和 html 类型共享智能检测逻辑
  const text = clip.text_content || "";

  // 0. URL 检测（纯 URL 文本 → 卡片预览，不需要 hljs/marked/DOMPurify）
  if (text.length > 0 && isUrl(text.trim())) {
    renderUrlCard(text.trim());
    return;
  }

  // 0.5 JSON 检测（纯 JSON 文本 → 格式化 + 语法高亮）
  if (text.length > 1 && isJson(text.trim())) {
    await ensureLibs();
    renderJson(text.trim());
    return;
  }

  const trimmed = text.trim();

  // 0.6 JWT 检测（必须在编码检测之前，否则被误识别为 Base64）
  if (trimmed.length > 30 && isJwt(trimmed)) {
    await ensureLibs();
    renderJwt(trimmed);
    return;
  }

  // 0.7 可逆编码检测（Base64/URL编码/HTML实体/Unicode/Hex）
  const encodingResult = detectEncoding(trimmed);
  if (encodingResult) {
    if (encodingResult.type === "base64-image") {
      renderBase64Image(encodingResult);
    } else {
      renderEncoded(encodingResult);
    }
    return;
  }

  // 0.8 哈希识别（不可逆，仅标注）
  const hashType = trimmed.length >= 32 && trimmed.length <= 200 && identifyHash(trimmed);
  if (hashType) {
    renderHash(trimmed, hashType);
    return;
  }

  // 0.9 加密内容检测（允许输入密钥解密）
  const encryptType = trimmed.length >= 24 && detectEncrypted(trimmed);
  if (encryptType) {
    renderEncrypted(trimmed, encryptType);
    return;
  }

  // 0.10 颜色值检测
  if (trimmed.length <= 50 && isColor(trimmed)) {
    renderColor(trimmed);
    return;
  }

  // 0.11 Unix 时间戳检测
  if (isTimestamp(trimmed)) {
    renderTimestamp(trimmed);
    return;
  }

  // 0.12 UUID 检测
  if (trimmed.length >= 32 && trimmed.length <= 39 && isUuid(trimmed)) {
    renderUuid(trimmed);
    return;
  }

  // 0.13 IP 地址检测
  if (trimmed.length >= 7 && trimmed.length <= 45 && isIpAddress(trimmed)) {
    renderIpAddress(trimmed);
    return;
  }

  // 0.14 Email 地址检测
  if (trimmed.length >= 5 && trimmed.length <= 254 && isEmail(trimmed)) {
    renderEmail(trimmed);
    return;
  }

  // 0.15 MAC 地址检测
  if (trimmed.length >= 17 && trimmed.length <= 23 && isMacAddress(trimmed)) {
    renderMac(trimmed);
    return;
  }

  // 0.16 Cron 表达式检测
  if (trimmed.length >= 9 && isCron(trimmed)) {
    renderCron(trimmed);
    return;
  }

  // 0.17 日期字符串检测
  if (trimmed.length >= 8 && trimmed.length <= 40 && isDateString(trimmed)) {
    renderDate(trimmed);
    return;
  }

  // 0.18 语义版本号检测
  if (trimmed.length >= 5 && trimmed.length <= 60 && isSemver(trimmed)) {
    renderSemver(trimmed);
    return;
  }

  // 0.19 数字进制检测
  if (trimmed.length >= 3 && trimmed.length <= 66 && isNumberBase(trimmed)) {
    renderNumberBase(trimmed);
    return;
  }

  // 0.20 CSS 渐变检测
  if (trimmed.length >= 20 && isGradient(trimmed)) {
    renderGradient(trimmed);
    return;
  }

  // 0.21 数据大小检测
  if (trimmed.length >= 2 && trimmed.length <= 20 && isDataSize(trimmed)) {
    renderDataSize(trimmed);
    return;
  }

  // 0.22 正则表达式检测
  if (trimmed.length >= 3 && trimmed.startsWith("/") && isRegex(trimmed)) {
    renderRegex(trimmed);
    return;
  }

  // 0.23 坐标检测
  if (trimmed.length >= 5 && trimmed.length <= 40 && isCoordinate(trimmed)) {
    renderCoordinate(trimmed);
    return;
  }

  // 0.24 MIME type 检测
  if (trimmed.length >= 3 && trimmed.length <= 100 && trimmed.includes("/") && isMimeType(trimmed)) {
    renderMimeType(trimmed);
    return;
  }

  // 0.25 数学表达式检测
  if (trimmed.length >= 3 && trimmed.length <= 100 && isMathExpr(trimmed)) {
    renderMathExpr(trimmed);
    return;
  }

  // 0.26 HTTP 状态码检测
  if (trimmed.length === 3 && isHttpStatus(trimmed)) {
    renderHttpStatus(trimmed);
    return;
  }

  // 延迟加载渲染库（首次调用时初始化 hljs/marked/DOMPurify）
  await ensureLibs();

  // 1. Markdown 检测（优先，评分制，需多个特征）
  if (text.length > 0 && isMarkdown(text)) {
    renderMarkdown(text);
    return;
  }

  // 2. 代码检测（text_content 纯文本，排除 xml 误判）
  if (text.length > 10) {
    const cacheKey = clip.content_hash;
    let result = cacheKey && _hljsCache.get(cacheKey);
    if (!result) {
      result = hljs.highlightAuto(text);
      if (cacheKey) {
        if (_hljsCache.size >= HLJS_CACHE_MAX) {
          const oldest = _hljsCache.keys().next().value;
          _hljsCache.delete(oldest);
        }
        _hljsCache.set(cacheKey, { language: result.language, relevance: result.relevance, value: result.value });
      }
    }
    if (result.relevance > CODE_RELEVANCE_THRESHOLD && result.language && result.language !== "xml") {
      renderCode(text, result);
      return;
    }
  }

  // 3. HTML 富文本渲染（仅 html 类型，按需加载 html_content）
  if (clip.content_type === "html") {
    try {
      const detail = await getClipDetail(clip.id);
      if (_currentClipId !== clip.id) return; // 异步期间焦点已切换
      if (detail.html_content) {
        renderRichText(detail.html_content);
        return;
      }
    } catch (_) { /* 回退到纯文本 */ }
  }

  // 4. 纯文本
  renderPlainText(text);
}

function renderCode(text, result) {
  _badgeEl.textContent = result.language.toUpperCase();
  _contentEl.classList.add("preview-content--code");
  const pre = document.createElement("pre");
  const code = document.createElement("code");
  code.className = `hljs language-${result.language}`;
  code.innerHTML = DOMPurify.sanitize(result.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
  pre.appendChild(code);
  _contentEl.appendChild(pre);
}

function renderJson(text) {
  _badgeEl.textContent = "JSON";
  _contentEl.classList.add("preview-content--code");
  let formatted;
  try { formatted = JSON.stringify(JSON.parse(text), null, 2); } catch { formatted = text; }
  const highlighted = hljs.highlight(formatted, { language: "json" });
  const pre = document.createElement("pre");
  const code = document.createElement("code");
  code.className = "hljs language-json";
  code.innerHTML = DOMPurify.sanitize(highlighted.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
  pre.appendChild(code);
  _contentEl.appendChild(pre);
}

// ── 编码 / 哈希 / 加密 渲染函数 ────────────────────────────

/** 可逆编码对照渲染 */
function renderEncoded(result) {
  const LABELS = {
    "base64": "BASE64", "url-encoded": "URL ENCODED",
    "html-entity": "HTML ENTITY", "unicode": "UNICODE", "hex": "HEX",
  };
  _badgeEl.textContent = LABELS[result.type] || result.type.toUpperCase();
  _contentEl.classList.add("preview-content--encoded");

  // 解码结果
  const decodedSection = document.createElement("div");
  decodedSection.className = "encoded-section encoded-decoded";
  const decodedLabel = document.createElement("div");
  decodedLabel.className = "encoded-label";
  decodedLabel.textContent = t("preview.decoded") || "Decoded";
  const decodedBox = document.createElement("pre");
  decodedBox.className = "encoded-box";
  decodedBox.textContent = result.decoded;
  decodedSection.append(decodedLabel, decodedBox);

  // 原文（可折叠）
  const originalSection = document.createElement("details");
  originalSection.className = "encoded-section encoded-original";
  const summary = document.createElement("summary");
  summary.className = "encoded-label encoded-toggle";
  summary.textContent = t("preview.original") || "Original";
  const originalBox = document.createElement("pre");
  originalBox.className = "encoded-box encoded-box--muted";
  originalBox.textContent = result.original;
  originalSection.append(summary, originalBox);

  _contentEl.append(decodedSection, originalSection);
}

/** Base64 图片渲染 */
function renderBase64Image(result) {
  _badgeEl.textContent = "BASE64 → IMAGE";
  _contentEl.classList.add("preview-content--image");
  const img = document.createElement("img");
  // 通过魔数推断格式
  const d = result.decoded;
  let mime = "image/png";
  if (d.startsWith("\xFF\xD8\xFF")) mime = "image/jpeg";
  else if (d.startsWith("GIF8")) mime = "image/gif";
  else if (d.startsWith("RIFF")) mime = "image/webp";
  img.src = `data:${mime};base64,${result.original.replace(/[\s\r\n]/g, "")}`;
  img.alt = "Base64 decoded image";
  _contentEl.appendChild(img);
}

/** JWT 结构化渲染 */
function renderJwt(text) {
  _badgeEl.textContent = "JWT";
  _contentEl.classList.add("preview-content--encoded");
  const { header, payload, signature } = parseJwt(text);

  // Header
  if (header) {
    const sec = _jwtSection("Header", JSON.stringify(header, null, 2));
    _contentEl.appendChild(sec);
  }
  // Payload
  if (payload) {
    const sec = _jwtSection("Payload", JSON.stringify(payload, null, 2));
    _contentEl.appendChild(sec);
  }
  // Signature
  const sigSec = document.createElement("div");
  sigSec.className = "encoded-section jwt-signature";
  const sigLabel = document.createElement("div");
  sigLabel.className = "encoded-label encoded-label--warn";
  sigLabel.textContent = "⚠ Signature (not verified)";
  const sigBox = document.createElement("pre");
  sigBox.className = "encoded-box encoded-box--muted";
  sigBox.textContent = signature;
  sigSec.append(sigLabel, sigBox);
  _contentEl.appendChild(sigSec);
}

function _jwtSection(label, jsonText) {
  const sec = document.createElement("div");
  sec.className = "encoded-section";
  const lbl = document.createElement("div");
  lbl.className = "encoded-label";
  lbl.textContent = label;
  sec.appendChild(lbl);

  const pre = document.createElement("pre");
  const code = document.createElement("code");
  code.className = "hljs language-json";
  const highlighted = hljs.highlight(jsonText, { language: "json" });
  code.innerHTML = DOMPurify.sanitize(highlighted.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
  pre.appendChild(code);
  sec.appendChild(pre);
  return sec;
}

/** 哈希识别渲染 */
function renderHash(text, hashType) {
  _badgeEl.textContent = `HASH · ${hashType}`;
  _contentEl.classList.add("preview-content--text");
  const mono = document.createElement("pre");
  mono.className = "encoded-box";
  mono.textContent = text;
  _contentEl.appendChild(mono);
  const hint = document.createElement("div");
  hint.className = "encoded-hint";
  hint.textContent = t("preview.hashHint") || "Irreversible hash — cannot be decoded";
  _contentEl.appendChild(hint);
}

/** 颜色值渲染：色块 + 格式信息 */
function renderColor(text) {
  _badgeEl.textContent = "COLOR";
  _contentEl.classList.add("preview-content--encoded");

  const wrapper = document.createElement("div");
  wrapper.className = "color-preview";

  // 大色块
  const swatch = document.createElement("div");
  swatch.className = "color-swatch";
  swatch.style.backgroundColor = normalizeColor(text);
  wrapper.appendChild(swatch);

  // 颜色值
  const value = document.createElement("div");
  value.className = "color-value";
  value.textContent = text;
  wrapper.appendChild(value);

  // 尝试展示转换后的其他格式
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = normalizeColor(text);
  ctx.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
  const hex = `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
  const rgb = `rgb(${r}, ${g}, ${b})`;

  const alts = document.createElement("div");
  alts.className = "color-alts";
  if (!text.startsWith("#")) {
    const hexEl = document.createElement("span");
    hexEl.className = "color-alt";
    hexEl.textContent = a < 255 ? `${hex} (α ${(a / 255).toFixed(2)})` : hex;
    alts.appendChild(hexEl);
  }
  if (!text.toLowerCase().startsWith("rgb")) {
    const rgbEl = document.createElement("span");
    rgbEl.className = "color-alt";
    rgbEl.textContent = a < 255 ? `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(2)})` : rgb;
    alts.appendChild(rgbEl);
  }
  wrapper.appendChild(alts);

  // 对比色显示文字
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  const contrastText = document.createElement("div");
  contrastText.className = "color-contrast";
  contrastText.textContent = lum > 0.5 ? "Dark text recommended" : "Light text recommended";
  contrastText.style.color = lum > 0.5 ? "#333" : "#eee";
  contrastText.style.backgroundColor = normalizeColor(text);
  wrapper.appendChild(contrastText);

  _contentEl.appendChild(wrapper);
}

/** Unix 时间戳渲染 */
function renderTimestamp(text) {
  _badgeEl.textContent = "TIMESTAMP";
  _contentEl.classList.add("preview-content--encoded");
  const info = formatTimestamp(text);

  const wrapper = document.createElement("div");
  wrapper.className = "timestamp-preview";

  const rows = [
    [t("preview.tsLocal") || "Local", info.local],
    ["UTC", info.utc],
    [t("preview.tsRelative") || "Relative", info.relative],
    [t("preview.tsPrecision") || "Precision", info.precision === "ms" ? "Milliseconds" : "Seconds"],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    wrapper.appendChild(row);
  }

  // 原始值
  const orig = document.createElement("div");
  orig.className = "encoded-box encoded-box--muted";
  orig.textContent = text;
  wrapper.appendChild(orig);

  _contentEl.appendChild(wrapper);
}

/** UUID 渲染 */
function renderUuid(text) {
  const ver = uuidVersion(text);
  _badgeEl.textContent = ver ? `UUID ${ver}` : "UUID";
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  if (ver) {
    const hint = document.createElement("div");
    hint.className = "encoded-hint";
    const descs = {
      v1: "Time-based (MAC address + timestamp)",
      v2: "DCE Security",
      v3: "Name-based (MD5)",
      v4: "Random",
      v5: "Name-based (SHA-1)",
      v7: "Unix Epoch time-ordered",
    };
    hint.textContent = descs[ver] || `Version ${ver}`;
    _contentEl.appendChild(hint);
  }
}

/** IP 地址渲染 */
function renderIpAddress(text) {
  const info = ipInfo(text);
  _badgeEl.textContent = info.version;
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "ip-details";
  const items = [
    [t("preview.ipType") || "Type", info.type],
    [t("preview.ipVersion") || "Version", info.version],
  ];
  if (info.cidr) items.push(["CIDR", "Yes"]);
  for (const [label, value] of items) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** Email 渲染 */
function renderEmail(text) {
  const info = emailInfo(text);
  _badgeEl.textContent = "EMAIL";
  _contentEl.classList.add("preview-content--encoded");

  const link = document.createElement("a");
  link.href = `mailto:${text}`;
  link.className = "email-link";
  link.textContent = text;
  link.addEventListener("click", (e) => e.preventDefault());
  _contentEl.appendChild(link);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.emailLocal") || "Local", info.local],
    [t("preview.emailDomain") || "Domain", info.domain],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** MAC 地址渲染 */
function renderMac(text) {
  const info = macInfo(text);
  _badgeEl.textContent = `MAC · ${info.format}`;
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = info.normalized;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.macFormat") || "Format", info.format],
    ["OUI", info.oui],
    [t("preview.macType") || "Type", info.localAdmin ? "Locally Administered" : "Universally Administered"],
    [t("preview.macCast") || "Cast", info.multicast ? "Multicast" : "Unicast"],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** Cron 表达式渲染 */
function renderCron(text) {
  const info = cronDescribe(text);
  _badgeEl.textContent = `CRON · ${info.fields} fields`;
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  const fields = text.trim().split(/\s+/);
  const labels = info.fields === 6
    ? ["Second", "Minute", "Hour", "Day", "Month", "Weekday"]
    : ["Minute", "Hour", "Day", "Month", "Weekday"];
  const table = document.createElement("div");
  table.className = "cron-fields";
  for (let i = 0; i < fields.length; i++) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = labels[i];
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = fields[i];
    row.append(lbl, val);
    table.appendChild(row);
  }
  _contentEl.appendChild(table);

  if (info.description) {
    const hint = document.createElement("div");
    hint.className = "encoded-hint";
    hint.textContent = info.description;
    _contentEl.appendChild(hint);
  }
}

/** 日期字符串渲染 */
function renderDate(text) {
  const info = dateInfo(text);
  _badgeEl.textContent = "DATE";
  _contentEl.classList.add("preview-content--encoded");

  const wrapper = document.createElement("div");
  wrapper.className = "timestamp-preview";

  const rows = [
    [t("preview.tsLocal") || "Local", info.local],
    ["UTC", info.utc],
    ["ISO 8601", info.iso],
    ["Unix Timestamp", String(info.timestamp)],
    [t("preview.tsRelative") || "Relative", info.relative],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    wrapper.appendChild(row);
  }

  const orig = document.createElement("div");
  orig.className = "encoded-box encoded-box--muted";
  orig.textContent = text;
  wrapper.appendChild(orig);

  _contentEl.appendChild(wrapper);
}

/** 语义版本号渲染 */
function renderSemver(text) {
  const info = semverInfo(text);
  _badgeEl.textContent = "SEMVER";
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = info.normalized;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    ["Major", String(info.major)],
    ["Minor", String(info.minor)],
    ["Patch", String(info.patch)],
  ];
  if (info.preRelease) rows.push(["Pre-release", info.preRelease]);
  if (info.build) rows.push(["Build", info.build]);
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** 数字进制渲染 */
function renderNumberBase(text) {
  const info = numberBaseInfo(text);
  const baseNames = { 2: "BIN", 8: "OCT", 16: "HEX" };
  _badgeEl.textContent = `NUMBER · ${baseNames[info.base] || `BASE${info.base}`}`;
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.numDecimal") || "Decimal", String(info.decimal)],
    [t("preview.numHex") || "Hex", info.hex],
    [t("preview.numBin") || "Binary", info.binary],
    [t("preview.numOct") || "Octal", info.octal],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** CSS 渐变渲染 */
function renderGradient(text) {
  _badgeEl.textContent = "GRADIENT";
  _contentEl.classList.add("preview-content--encoded");

  // 可视化预览色块
  const swatch = document.createElement("div");
  swatch.className = "gradient-swatch";
  swatch.style.background = text.trim();
  _contentEl.appendChild(swatch);

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text.trim();
  _contentEl.appendChild(box);
}

/** 数据大小渲染 */
function renderDataSize(text) {
  const info = dataSizeInfo(text);
  _badgeEl.textContent = "DATA SIZE";
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text.trim();
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  for (const conv of info.conversions) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = conv;
    row.appendChild(val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** 正则表达式渲染 */
function renderRegex(text) {
  const info = regexInfo(text);
  _badgeEl.textContent = "REGEX";
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.regexPattern") || "Pattern", info.pattern],
    [t("preview.regexFlags") || "Flags", info.flags || "(none)"],
  ];
  if (info.flagDescs.length > 0) {
    rows.push([t("preview.regexDesc") || "Description", info.flagDescs.join(", ")]);
  }
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** 坐标渲染 */
function renderCoordinate(text) {
  const info = coordInfo(text);
  _badgeEl.textContent = "COORDINATE";
  _contentEl.classList.add("preview-content--encoded");

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.coordDecimal") || "Decimal", info.decimal],
    ["DMS", info.dms],
    [t("preview.coordLat") || "Latitude", `${info.lat >= 0 ? "N" : "S"} ${Math.abs(info.lat).toFixed(6)}°`],
    [t("preview.coordLng") || "Longitude", `${info.lng >= 0 ? "E" : "W"} ${Math.abs(info.lng).toFixed(6)}°`],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** MIME type 渲染 */
function renderMimeType(text) {
  const info = mimeInfo(text);
  _badgeEl.textContent = "MIME TYPE";
  _contentEl.classList.add("preview-content--encoded");

  const box = document.createElement("pre");
  box.className = "encoded-box";
  box.textContent = text;
  _contentEl.appendChild(box);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.mimeType") || "Type", info.type],
    [t("preview.mimeSubtype") || "Subtype", info.subtype],
    [t("preview.mimeDesc") || "Description", info.description],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** 数学表达式渲染 */
function renderMathExpr(text) {
  const info = mathEval(text);
  _badgeEl.textContent = "MATH";
  _contentEl.classList.add("preview-content--encoded");

  const exprBox = document.createElement("pre");
  exprBox.className = "encoded-box";
  exprBox.textContent = `${text} = ${info.result}`;
  _contentEl.appendChild(exprBox);

  const resultBox = document.createElement("div");
  resultBox.className = "math-result";
  resultBox.textContent = String(info.result);
  _contentEl.appendChild(resultBox);
}

/** HTTP 状态码渲染 */
function renderHttpStatus(text) {
  const info = httpStatusInfo(text);
  _badgeEl.textContent = `HTTP ${info.code}`;
  _contentEl.classList.add("preview-content--encoded");

  const header = document.createElement("div");
  header.className = "http-status-header";
  header.textContent = `${info.code} ${info.message}`;
  _contentEl.appendChild(header);

  const details = document.createElement("div");
  details.className = "timestamp-preview";
  const rows = [
    [t("preview.httpCategory") || "Category", info.category],
    [t("preview.httpMessage") || "Message", info.message],
  ];
  for (const [label, value] of rows) {
    const row = document.createElement("div");
    row.className = "timestamp-row";
    const lbl = document.createElement("span");
    lbl.className = "timestamp-label";
    lbl.textContent = label;
    const val = document.createElement("span");
    val.className = "timestamp-value";
    val.textContent = value;
    row.append(lbl, val);
    details.appendChild(row);
  }
  _contentEl.appendChild(details);
}

/** 加密内容渲染 + 密钥输入解密 */
function renderEncrypted(text, encType) {
  _badgeEl.textContent = `ENCRYPTED · ${encType}`;
  _contentEl.classList.add("preview-content--encoded");

  // 加密标识
  const lockIcon = document.createElement("div");
  lockIcon.className = "encrypted-header";
  lockIcon.textContent = t("preview.encryptedHint") || "🔒 Encrypted content detected";
  _contentEl.appendChild(lockIcon);

  // PGP / ENCRYPTED-KEY 不提供解密 UI
  if (encType === "PGP" || encType === "ENCRYPTED-KEY") {
    const box = document.createElement("pre");
    box.className = "encoded-box encoded-box--muted";
    box.textContent = text;
    _contentEl.appendChild(box);
    const hint = document.createElement("div");
    hint.className = "encoded-hint";
    hint.textContent = t("preview.pgpHint") || "Use gpg or ssh-keygen to decrypt";
    _contentEl.appendChild(hint);
    return;
  }

  // 解密表单
  const form = document.createElement("div");
  form.className = "decrypt-form";

  // 算法选择
  const algoRow = _formRow(t("preview.algorithm") || "Algorithm");
  const algoSelect = document.createElement("select");
  algoSelect.className = "decrypt-select";
  const algos = encType === "AES-OpenSSL"
    ? ["AES-256-CBC", "AES-128-CBC"]
    : ["AES-256-CBC", "AES-128-CBC", "AES-256-GCM", "AES-128-GCM"];
  for (const a of algos) {
    const opt = document.createElement("option");
    opt.value = a;
    opt.textContent = a;
    algoSelect.appendChild(opt);
  }
  algoRow.appendChild(algoSelect);
  form.appendChild(algoRow);

  // OpenSSL PBKDF2 模式提示
  if (encType === "AES-OpenSSL") {
    const note = document.createElement("div");
    note.className = "encoded-hint";
    note.textContent = t("preview.opensslNote") || "Only supports openssl enc -pbkdf2 format";
    form.appendChild(note);
  }

  // 密码/密钥
  const keyRow = _formRow(encType === "AES-OpenSSL"
    ? (t("preview.password") || "Password")
    : (t("preview.key") || "Key (hex)"));
  const keyInput = document.createElement("input");
  keyInput.type = encType === "AES-OpenSSL" ? "password" : "text";
  keyInput.className = "decrypt-input";
  keyInput.placeholder = encType === "AES-OpenSSL" ? "passphrase" : "hex key";
  keyRow.appendChild(keyInput);
  form.appendChild(keyRow);

  // IV 输入（非 OpenSSL 格式需要）
  let ivInput = null;
  if (encType !== "AES-OpenSSL") {
    const ivRow = _formRow("IV (hex)");
    ivInput = document.createElement("input");
    ivInput.type = "text";
    ivInput.className = "decrypt-input";
    ivInput.placeholder = "initialization vector (hex)";
    ivRow.appendChild(ivInput);
    form.appendChild(ivRow);
  }

  // 解密按钮
  const btn = document.createElement("button");
  btn.className = "decrypt-btn";
  btn.textContent = t("preview.decrypt") || "🔓 Decrypt";
  form.appendChild(btn);
  _contentEl.appendChild(form);

  // 密文展示
  const cipherSection = document.createElement("details");
  cipherSection.className = "encoded-section";
  const cipherSummary = document.createElement("summary");
  cipherSummary.className = "encoded-label encoded-toggle";
  cipherSummary.textContent = t("preview.ciphertext") || "Ciphertext";
  const cipherBox = document.createElement("pre");
  cipherBox.className = "encoded-box encoded-box--muted";
  cipherBox.textContent = text;
  cipherSection.append(cipherSummary, cipherBox);
  _contentEl.appendChild(cipherSection);

  // 解密结果区
  const resultArea = document.createElement("div");
  resultArea.className = "decrypt-result";
  resultArea.hidden = true;
  _contentEl.appendChild(resultArea);

  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "⏳";
    resultArea.hidden = true;
    try {
      let decrypted;
      if (encType === "AES-OpenSSL") {
        decrypted = await decryptOpenSSL(text, keyInput.value, algoSelect.value);
      } else {
        decrypted = await decryptGeneric(text, keyInput.value, ivInput?.value || "", algoSelect.value);
      }
      resultArea.hidden = false;
      resultArea.innerHTML = "";
      const label = document.createElement("div");
      label.className = "encoded-label";
      label.textContent = t("preview.decrypted") || "Decrypted";
      const box = document.createElement("pre");
      box.className = "encoded-box";
      box.textContent = decrypted;
      resultArea.append(label, box);
    } catch (err) {
      resultArea.hidden = false;
      resultArea.innerHTML = "";
      const errEl = document.createElement("div");
      errEl.className = "decrypt-error";
      errEl.textContent = `❌ ${err.message || err}`;
      resultArea.appendChild(errEl);
    } finally {
      btn.disabled = false;
      btn.textContent = t("preview.decrypt") || "🔓 Decrypt";
    }
  });
}

function _formRow(label) {
  const row = document.createElement("div");
  row.className = "decrypt-row";
  const lbl = document.createElement("label");
  lbl.className = "decrypt-label";
  lbl.textContent = label;
  row.appendChild(lbl);
  return row;
}

/** OpenSSL 格式解密：Salted__ + PBKDF2 → AES (仅支持 openssl enc -pbkdf2) */
async function decryptOpenSSL(b64Text, password, algo) {
  const raw = Uint8Array.from(atob(b64Text), c => c.charCodeAt(0));
  // 前 8 字节 = "Salted__"，接下来 8 字节 = salt
  if (raw.length < 16) throw new Error("Invalid OpenSSL format");
  const magic = new TextDecoder().decode(raw.slice(0, 8));
  if (magic !== "Salted__") throw new Error("Missing OpenSSL Salted__ header");
  const salt = raw.slice(8, 16);
  const ciphertext = raw.slice(16);

  const keyLen = algo.includes("256") ? 32 : 16;
  const enc = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey("raw", enc.encode(password), "PBKDF2", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "PBKDF2", salt, iterations: 10000, hash: "SHA-256" },
    keyMaterial, (keyLen + 16) * 8
  );
  const derived = new Uint8Array(bits);
  const key = derived.slice(0, keyLen);
  const iv = derived.slice(keyLen, keyLen + 16);

  const cryptoKey = await crypto.subtle.importKey("raw", key, "AES-CBC", false, ["decrypt"]);
  const decrypted = await crypto.subtle.decrypt({ name: "AES-CBC", iv }, cryptoKey, ciphertext);
  return new TextDecoder().decode(decrypted);
}

/** 通用 AES 解密：用户提供 hex key + hex IV */
async function decryptGeneric(b64Text, hexKey, hexIV, algo) {
  const ciphertext = Uint8Array.from(atob(b64Text), c => c.charCodeAt(0));
  const key = hexToBytes(hexKey);
  const iv = hexToBytes(hexIV);

  const isGCM = algo.includes("GCM");
  const algoName = isGCM ? "AES-GCM" : "AES-CBC";
  const cryptoKey = await crypto.subtle.importKey("raw", key, algoName, false, ["decrypt"]);
  const params = isGCM ? { name: "AES-GCM", iv } : { name: "AES-CBC", iv };
  const decrypted = await crypto.subtle.decrypt(params, cryptoKey, ciphertext);
  return new TextDecoder().decode(decrypted);
}

function hexToBytes(hex) {
  const clean = hex.replace(/\s/g, "");
  if (clean.length === 0 || clean.length % 2 !== 0) throw new Error("Invalid hex length");
  if (!/^[0-9a-f]+$/i.test(clean)) throw new Error("Invalid hex characters");
  const bytes = new Uint8Array(clean.length / 2);
  for (let i = 0; i < clean.length; i += 2) {
    bytes[i / 2] = parseInt(clean.slice(i, i + 2), 16);
  }
  return bytes;
}

function renderMarkdown(text) {
  _badgeEl.textContent = "MARKDOWN";
  _contentEl.classList.add("preview-content--html");
  const rawHtml = marked.parse(text);
  _contentEl.innerHTML = DOMPurify.sanitize(rawHtml, PURIFY_CONFIG);
}

function renderRichText(html) {
  _badgeEl.textContent = "RICH TEXT";
  _contentEl.classList.add("preview-content--html");
  _contentEl.innerHTML = DOMPurify.sanitize(html, PURIFY_CONFIG);
}

function renderUrlCard(url) {
  _badgeEl.textContent = "URL";
  _contentEl.classList.add("preview-content--url");

  // 先渲染基础 URL 信息（立即显示）
  const card = document.createElement("div");
  card.className = "url-card";
  const urlDisplay = document.createElement("a");
  urlDisplay.className = "url-card-url";
  urlDisplay.textContent = url;
  urlDisplay.href = "#";
  urlDisplay.onclick = (e) => e.preventDefault();
  card.appendChild(urlDisplay);

  const loading = document.createElement("div");
  loading.className = "url-card-loading";
  loading.textContent = t("preview.urlLoading") || "Loading...";
  card.appendChild(loading);
  _contentEl.appendChild(card);

  // 异步抓取 OG 元数据
  fetchUrlMeta(url).then(meta => {
    if (_contentEl.querySelector(".url-card") !== card) return; // 已切换
    loading.remove();

    if (meta.favicon) {
      const icon = document.createElement("img");
      icon.className = "url-card-favicon";
      icon.src = meta.favicon;
      icon.width = 16;
      icon.height = 16;
      icon.onerror = () => icon.remove();
      card.insertBefore(icon, urlDisplay);
    }
    if (meta.title) {
      const title = document.createElement("h3");
      title.className = "url-card-title";
      title.textContent = meta.title;
      card.insertBefore(title, urlDisplay);
    }
    if (meta.description) {
      const desc = document.createElement("p");
      desc.className = "url-card-desc";
      desc.textContent = meta.description.slice(0, 200);
      card.insertBefore(desc, urlDisplay);
    }
    if (meta.site_name) {
      const site = document.createElement("span");
      site.className = "url-card-site";
      site.textContent = meta.site_name;
      card.insertBefore(site, urlDisplay);
    }
  }).catch(() => {
    loading.textContent = url;
  });
}

function renderPlainText(text) {
  _badgeEl.textContent = "TEXT";
  _contentEl.classList.add("preview-content--text");
  _contentEl.textContent = text;
}

async function renderImage(clip) {
  _badgeEl.textContent = "IMAGE";
  _contentEl.classList.add("preview-content--image");
  try {
    const base64 = await getClipImage(clip.id);
    if (base64) {
      const img = document.createElement("img");
      img.src = `data:image/png;base64,${base64}`;
      img.alt = "clipboard image";
      img.onload = () => {
        _metaEl.textContent = `${img.naturalWidth}×${img.naturalHeight} · ${
          clip.byte_size > 1024
            ? (clip.byte_size / 1024).toFixed(1) + " KB"
            : clip.byte_size + " B"
        }`;
      };
      _contentEl.appendChild(img);

      // 自动 OCR：在图片下方显示可选择的识别文字
      const ocrArea = document.createElement("div");
      ocrArea.className = "preview-ocr-result";
      const ocrText = document.createElement("pre");
      ocrArea.appendChild(ocrText);
      _contentEl.appendChild(ocrArea);

      // 检查 OCR 是否已启用
      try {
        const config = await getConfig();
        if (config.ocr_enabled === false) {
          ocrArea.style.display = "none";
          return;
        }
      } catch (_) { /* 读取配置失败则继续 */ }

      // 先检查 OCR 是否可用
      const available = await ocrAvailable().catch(() => false);
      if (!available) {
        ocrText.textContent = t("action.ocrUnavailable");
        ocrArea.dataset.status = "unavailable";
        return;
      }

      ocrArea.dataset.status = "loading";
      ocrText.textContent = t("action.ocrProcessing");

      // 异步识别
      ocrImage(clip.id).then(async (text) => {
        if (_currentClipId !== clip.id) return; // 焦点已切换
        if (text && text.trim()) {
          // 检查配置：clipboard 模式直接复制，preview 模式显示文字
          try {
            const config = await getConfig();
            if (config.ocr_result_mode === "clipboard") {
              await navigator.clipboard.writeText(text);
              ocrText.textContent = "✓ " + t("settings.ocr.clipboard");
              ocrArea.dataset.status = "done";
              return;
            }
          } catch (_) { /* 读取配置失败则默认 preview 模式 */ }
          ocrText.textContent = text;
          ocrArea.dataset.status = "done";
        } else {
          ocrText.textContent = t("action.ocrEmpty");
          ocrArea.dataset.status = "empty";
        }
      }).catch(() => {
        if (_currentClipId !== clip.id) return;
        ocrText.textContent = t("action.ocrFailed");
        ocrArea.dataset.status = "error";
      });
    }
  } catch (e) {
    _contentEl.textContent = t("preview.imageLoadFailed");
    console.warn("预览图片加载失败:", e);
  }
}

// ── OCR 文字区域焦点管理 ──

/** OCR 文字区域是否已聚焦 */
export function isOcrFocused() {
  const ocrPre = _contentEl.querySelector(".preview-ocr-result pre");
  return ocrPre && document.activeElement === ocrPre;
}

/** 聚焦 OCR 文字区域，全选文字并显示复制提示。返回是否成功聚焦 */
export function focusOcr() {
  const ocrArea = _contentEl.querySelector(".preview-ocr-result");
  if (!ocrArea || ocrArea.dataset.status !== "done") return false;
  const ocrPre = ocrArea.querySelector("pre");
  if (!ocrPre || !ocrPre.textContent.trim()) return false;
  ocrPre.setAttribute("tabindex", "0");
  ocrPre.focus();
  // 全选文字
  const range = document.createRange();
  range.selectNodeContents(ocrPre);
  const sel = window.getSelection();
  sel.removeAllRanges();
  sel.addRange(range);
  // 显示复制提示
  let hint = _contentEl.querySelector(".preview-ocr-hint");
  if (!hint) {
    hint = document.createElement("div");
    hint.className = "preview-ocr-hint";
    hint.textContent = t("action.ocrCopyHint");
    ocrPre.parentElement.appendChild(hint);
  }
  hint.hidden = false;
  return true;
}

/** 移除 OCR 文字区域焦点 */
export function blurOcr() {
  const ocrPre = _contentEl.querySelector(".preview-ocr-result pre");
  if (ocrPre) {
    ocrPre.blur();
    ocrPre.removeAttribute("tabindex");
    window.getSelection().removeAllRanges();
  }
  const hint = _contentEl.querySelector(".preview-ocr-hint");
  if (hint) hint.hidden = true;
}

// ── 测试用内部暴露 ──────────────────────────────────────────
export const __test__ = {
  isReadable, isJwt, parseJwt, isBase64, isUrlEncoded, isHtmlEntities,
  isUnicodeEscape, isHexEncoded, detectEncoding, identifyHash,
  detectEncrypted, isColor, isTimestamp, formatTimestamp,
  isUuid, uuidVersion, isIpAddress, ipInfo,
  isEmail, emailInfo, isMacAddress, macInfo,
  isCron, cronDescribe, isDateString, dateInfo,
  isSemver, semverInfo, isNumberBase, numberBaseInfo,
  isGradient, isDataSize, dataSizeInfo,
  isRegex, regexInfo, isCoordinate, coordInfo,
  isMimeType, mimeInfo, isMathExpr, mathEval,
  isHttpStatus, httpStatusInfo,
};
