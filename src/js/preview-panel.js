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
