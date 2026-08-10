/**
 * preview/identifier-detectors.js — 颜色、时间及常见标识符检测
 */

// ── 颜色值 / 时间戳 / UUID / IP 检测 ───────────────────────

/** 颜色值检测：#hex / rgb() / hsl() / oklch() */
const COLOR_HEX_RE = /^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const COLOR_FUNC_RE = /^(rgba?|hsla?|oklch|oklab|hwb)\(\s*[\d.,/%\s-]+\)$/i;
export function isColor(text) {
  return COLOR_HEX_RE.test(text) || COLOR_FUNC_RE.test(text);
}

/** 解析颜色为标准化 CSS 值（用于渲染色块） */
export function normalizeColor(text) {
  // 浏览器可直接渲染这些格式，返回原文即可
  return text;
}

/** Unix 时间戳检测（10位秒 / 13位毫秒） */
const TIMESTAMP_RE = /^\d{10,13}$/;
export function isTimestamp(text) {
  if (!TIMESTAMP_RE.test(text)) return false;
  const n = parseInt(text, 10);
  // 合理范围：2000-01-01 到 2100-01-01
  const secs = text.length === 13 ? n / 1000 : n;
  return secs >= 946684800 && secs <= 4102444800;
}

/** 时间戳转日期展示 */
export function formatTimestamp(text) {
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
export function isUuid(text) {
  return UUID_RE.test(text);
}

export function uuidVersion(text) {
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
export function isIpAddress(text) {
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

export function ipInfo(text) {
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
export function isEmail(text) {
  return text.length >= 5 && text.length <= 254 && !text.includes("\n") && EMAIL_RE.test(text);
}

export function emailInfo(text) {
  const [local, domain] = text.split("@");
  return { local, domain };
}

/** MAC 地址检测（EUI-48: XX:XX:XX:XX:XX:XX 或 XX-XX-XX-XX-XX-XX，EUI-64 也支持） */
const MAC48_RE = /^([0-9a-f]{2}[:\-]){5}[0-9a-f]{2}$/i;
const MAC64_RE = /^([0-9a-f]{2}[:\-]){7}[0-9a-f]{2}$/i;
const MAC_CISCO_RE = /^[0-9a-f]{4}\.[0-9a-f]{4}\.[0-9a-f]{4}$/i;
export function isMacAddress(text) {
  return MAC48_RE.test(text) || MAC64_RE.test(text) || MAC_CISCO_RE.test(text);
}

export function macInfo(text) {
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
export function isCron(text) {
  const parts = text.trim().split(/\s+/);
  if (parts.length < 5 || parts.length > 6) return false;
  return parts.every(p => CRON_FIELD_RE.test(p));
}

export function cronDescribe(text) {
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
export function isDateString(text) {
  if (text.length < 8 || text.length > 40) return false;
  if (ISO_DATE_RE.test(text) || RFC_DATE_RE.test(text) || COMMON_DATE_RE.test(text)) {
    const d = new Date(text);
    return !isNaN(d.getTime());
  }
  return false;
}

export function dateInfo(text) {
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
