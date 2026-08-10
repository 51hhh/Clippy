/**
 * preview/format-detectors.js — 常见数据格式与表达式检测
 */

// ── 语义版本 / 数字进制 / CSS 渐变 / 数据大小检测 ─────────

/** 语义版本号检测 (SemVer) */
const SEMVER_RE = /^v?(\d+)\.(\d+)\.(\d+)(?:-([a-zA-Z0-9]+(?:\.[a-zA-Z0-9]+)*))?(?:\+([a-zA-Z0-9]+(?:\.[a-zA-Z0-9]+)*))?$/;
export function isSemver(text) {
  return SEMVER_RE.test(text.trim());
}

export function semverInfo(text) {
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
export function isNumberBase(text) {
  return HEX_NUM_RE.test(text) || BIN_NUM_RE.test(text) || OCT_NUM_RE.test(text);
}

export function numberBaseInfo(text) {
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
export function isGradient(text) {
  return GRADIENT_RE.test(text.trim());
}

/** 数据大小检测与转换 */
const DATA_SIZE_RE = /^(\d+(?:\.\d+)?)\s*(B|KB|KiB|MB|MiB|GB|GiB|TB|TiB|PB|PiB|bytes?)$/i;
export function isDataSize(text) {
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

export function dataSizeInfo(text) {
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
export function isRegex(text) {
  if (!text.startsWith("/") || text.length < 3) return false;
  const m = text.match(REGEX_RE);
  if (!m) return false;
  try { new RegExp(m[1], m[2]); return true; } catch { return false; }
}

export function regexInfo(text) {
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
export function isCoordinate(text) {
  const m = text.match(COORD_RE);
  if (!m) return false;
  const lat = parseFloat(m[1]);
  const lng = parseFloat(m[2]);
  return lat >= -90 && lat <= 90 && lng >= -180 && lng <= 180;
}

export function coordInfo(text) {
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
export function isMimeType(text) {
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

export function mimeInfo(text) {
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
export function isMathExpr(text) {
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

export function mathEval(text) {
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
export function isHttpStatus(text) {
  if (!HTTP_STATUS_RE.test(text.trim())) return false;
  return !!_HTTP_CODES[parseInt(text.trim())];
}

export function httpStatusInfo(code) {
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

export function isMarkdown(text) {
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
