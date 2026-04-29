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
 */

import { getClipImage, getClipDetail, setPreviewVisible, ocrImage } from "./api.js";
import { t } from "../i18n/i18n.js";
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import { marked } from "marked";

// ── highlight.js：按需注册常用语言 ──
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import python     from "highlight.js/lib/languages/python";
import java       from "highlight.js/lib/languages/java";
import cpp        from "highlight.js/lib/languages/cpp";
import c          from "highlight.js/lib/languages/c";
import csharp     from "highlight.js/lib/languages/csharp";
import go         from "highlight.js/lib/languages/go";
import rust       from "highlight.js/lib/languages/rust";
import bash       from "highlight.js/lib/languages/bash";
import sql        from "highlight.js/lib/languages/sql";
import json       from "highlight.js/lib/languages/json";
import xml        from "highlight.js/lib/languages/xml";
import cssLang    from "highlight.js/lib/languages/css";
import yaml       from "highlight.js/lib/languages/yaml";
import markdownLang from "highlight.js/lib/languages/markdown";
import ruby       from "highlight.js/lib/languages/ruby";
import php        from "highlight.js/lib/languages/php";
import swift      from "highlight.js/lib/languages/swift";
import kotlin     from "highlight.js/lib/languages/kotlin";
import lua        from "highlight.js/lib/languages/lua";

hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("java", java);
hljs.registerLanguage("cpp", cpp);
hljs.registerLanguage("c", c);
hljs.registerLanguage("csharp", csharp);
hljs.registerLanguage("go", go);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("json", json);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("css", cssLang);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("markdown", markdownLang);
hljs.registerLanguage("ruby", ruby);
hljs.registerLanguage("php", php);
hljs.registerLanguage("swift", swift);
hljs.registerLanguage("kotlin", kotlin);
hljs.registerLanguage("lua", lua);

// ── DOMPurify 配置 ──
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

// DOMPurify hook：清理背景色等主题冲突样式（仅影响预览渲染，不改原始数据）
const STYLE_REMOVE_RE = /\b(background-color|background(?!-)|color|position|z-index|opacity)\s*:[^;]*(;|$)/gi;
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

// ── Marked 配置（代码块用 highlight.js） ──
marked.setOptions({
  breaks: true,
  gfm: true,
});

// 自定义 renderer：代码块使用 hljs
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
  code.innerHTML = DOMPurify.sanitize(result.value);
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
      ocrArea.dataset.status = "loading";
      const ocrText = document.createElement("pre");
      ocrText.textContent = t("action.ocrProcessing");
      ocrArea.appendChild(ocrText);
      _contentEl.appendChild(ocrArea);

      // 异步识别
      ocrImage(clip.id).then(text => {
        if (_currentClipId !== clip.id) return; // 焦点已切换
        if (text && text.trim()) {
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
