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
import * as detectors from "./preview/detectors.js";

const {
  isUrl, isJson, isJwt, parseJwt, detectEncoding, identifyHash,
  detectEncrypted, isColor, normalizeColor, isTimestamp, formatTimestamp,
  isUuid, uuidVersion, isIpAddress, ipInfo, isEmail, emailInfo,
  isMacAddress, macInfo, isCron, cronDescribe, isDateString, dateInfo,
  isSemver, semverInfo, isNumberBase, numberBaseInfo, isGradient,
  isDataSize, dataSizeInfo, isRegex, regexInfo, isCoordinate, coordInfo,
  isMimeType, mimeInfo, isMathExpr, mathEval, isHttpStatus,
  httpStatusInfo, isMarkdown,
} = detectors;

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
export const __test__ = detectors;
