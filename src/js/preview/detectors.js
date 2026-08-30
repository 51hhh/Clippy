/**
 * preview/detectors.js — URL、结构化文本、编码、令牌与加密内容检测
 *
 * 该模块只负责检测和解析，不包含预览渲染或面板状态。
 */

export * from "./identifier-detectors.js";
export * from "./format-detectors.js";

import { decodeHtmlEntities } from "../html-entities.js";

// URL 检测（仅匹配单行纯 URL 文本）
const URL_RE = /^https?:\/\/[^\s]+$/;
export function isUrl(text) {
  return URL_RE.test(text) && text.length < 2048;
}

// JSON 检测：首字符为 { 或 [，尝试解析
export function isJson(text) {
  const c = text[0];
  if (c !== "{" && c !== "[") return false;
  try { JSON.parse(text); return true; } catch { return false; }
}

// ── 编码 / 哈希 / 加密检测 ──────────────────────────────────

/**
 * 字节串按 UTF-8 严格解码；不是合法 UTF-8 就返回 null。
 *
 * `atob` 和 hex 解码都产出"每个字符一个字节"的 Latin-1 字符串。直接拿它判可读性
 * 会把随机字节当成文本：单字节 0xA0-0xFF 在 Latin-1 里全是"正常字符"，
 * 24 字节随机数据里通常有七八成落在这个范围，比例阈值根本拦不住。
 * 真正编码过的文本几乎一定是 UTF-8，而随机字节几乎一定不是合法 UTF-8
 * （高位字节必须成对/成组出现且首字节决定长度），所以这一层比比例阈值可靠得多。
 */
function bytesToText(binary) {
  try {
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch { return null; }
}

/**
 * 字节串是否解出了人能读的文本：先要求是合法 UTF-8，再看可打印比例。
 * 返回解码后的文本（成功）或 null（失败），因此调用方直接拿它当展示内容，
 * 不必再解一遍——顺带修掉了 UTF-8 内容被按 Latin-1 显示成乱码的老问题。
 */
export function decodeReadableBytes(binary) {
  const text = bytesToText(binary);
  if (text === null) return null;
  return isReadable(text) ? text : null;
}

/** 可打印文本比例检测（输入是已解码的 Unicode 文本，不是字节串） */
export function isReadable(decoded) {
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
export function isJwt(text) {
  if (!text.startsWith("eyJ")) return false;
  const parts = text.split(".");
  if (parts.length !== 3) return false;
  return parts.every(p => BASE64URL_PART_RE.test(p));
}

/** 解析 JWT → { header, payload, signature } */
export function parseJwt(text) {
  const [h, p, s] = text.split(".");
  let header, payload;
  try { header = JSON.parse(base64urlDecode(h)); } catch { header = null; }
  try { payload = JSON.parse(base64urlDecode(p)); } catch { payload = null; }
  return { header, payload, signature: s };
}

/** Base64 检测 */
const BASE64_RE = /^[A-Za-z0-9+/]{24,}={0,2}$/;
export function isBase64(text) {
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
    // 图片魔数按原始字节判（上面那段），文本走严格 UTF-8：
    // 纯 hex 的哈希（MD5/SHA-1/SHA-256…）同时也是合法 Base64 字符集，
    // 按 Latin-1 判可读性会让它在这里被认成 Base64 并显示成乱码，
    // 于是永远走不到后面的 hash 规则。
    const readable = decodeReadableBytes(decoded);
    if (readable !== null) {
      return { type: "base64", decoded: readable, original: text };
    }
    return false;
  } catch { return false; }
}

/** URL 编码检测 */
const URL_ENCODED_RE = /%[0-9A-Fa-f]{2}/g;
export function isUrlEncoded(text) {
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
export function isHtmlEntities(text) {
  const matches = text.match(HTML_ENTITY_RE);
  if (!matches || matches.length < 2) return false;
  const decoded = decodeHtmlEntities(text);
  if (decoded === text) return false;
  return { type: "html-entity", decoded, original: text };
}

/** Unicode 转义检测 */
const UNICODE_RE = /\\u[0-9a-fA-F]{4}/g;
export function isUnicodeEscape(text) {
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
export function isHexEncoded(text) {
  const clean = text.startsWith("0x") || text.startsWith("0X") ? text.slice(2) : text;
  if (clean.length < 8 || clean.length % 2 !== 0 || clean.length > 2000) return false;
  if (!HEX_RE.test(text)) return false;
  // 不再按长度黑名单排除 32/40/64/128（哈希长度）：那样会把真正 hex 编码过的
  // 可读文本（正好 16/20/32/64 字节）也一并推给 hash 规则，标成 MD5/SHA-1。
  // 哈希的字节是随机的，几乎不可能是合法 UTF-8，下面的可读性判断足以分开两者。
  try {
    const chars = [];
    for (let i = 0; i < clean.length; i += 2) {
      chars.push(String.fromCharCode(parseInt(clean.slice(i, i + 2), 16)));
    }
    const readable = decodeReadableBytes(chars.join(""));
    if (readable !== null) {
      return { type: "hex", decoded: readable, original: text };
    }
    return false;
  } catch { return false; }
}

/** 统一编码检测入口 */
export function detectEncoding(text) {
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
export function identifyHash(text) {
  for (const { re, name } of HASH_PATTERNS) {
    if (re.test(text)) return name;
  }
  return null;
}

/** 加密内容检测 */
const OPENSSL_PREFIX = "U2FsdGVkX1"; // Base64 of "Salted__"
export function detectEncrypted(text) {
  if (text.startsWith("-----BEGIN PGP MESSAGE-----")) return "PGP";
  if (text.startsWith("-----BEGIN ENCRYPTED PRIVATE KEY-----")) return "ENCRYPTED-KEY";
  if (text.startsWith(OPENSSL_PREFIX)) return "AES-OpenSSL";
  // 通用加密检测：长 Base64 + 解码后不可读 + 高随机性
  const clean = text.replace(/[\s\r\n]/g, "");
  if (clean.length >= 64 && clean.length % 4 === 0 && BASE64_RE.test(clean)) {
    try {
      const decoded = atob(clean);
      if (decodeReadableBytes(decoded) === null && decoded.length >= 48 && decoded.length % 16 === 0) {
        return "AES-Generic";
      }
    } catch { /* not base64 */ }
  }
  return null;
}
