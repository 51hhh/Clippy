/**
 * preview-panel.js — 富文本预览面板
 *
 * 渲染优先级（text 和 html 类型共享检测逻辑）：
 * 1. highlight.js 语言检测 → relevance > 7 → 代码高亮（badge: 语言名）
 * 2. Markdown 正则匹配 → marked 渲染（badge: MARKDOWN）
 * 3. html 类型 → DOMPurify 安全渲染（badge: RICH TEXT）
 * 4. 纯文本 → 原样显示（badge: TEXT）
 * 5. image → base64 PNG（badge: IMAGE）
 *
 * 安全：剪贴板原始数据不可修改，所有处理仅在预览渲染层面。
 */

import { getClipImage, setPreviewVisible } from "./api.js";
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
const STYLE_REMOVE_RE = /\b(background-color|background|color)\s*:[^;]*(;|$)/gi;
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
  return `<pre><code class="hljs">${highlighted}</code></pre>`;
};
marked.use({ renderer });

// Markdown 首行特征正则
const MD_PATTERN = /^(#{1,6}\s|[-*+]\s|\d+\.\s|>\s|```|---|\*\*|__|\[.+\]\(|!\[)/m;

// 代码检测阈值
const CODE_RELEVANCE_THRESHOLD = 7;

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

export async function updatePreview(clip) {
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

  // 1. 代码检测（text_content 纯文本）
  if (text.length > 0) {
    const result = hljs.highlightAuto(text);
    if (result.relevance > CODE_RELEVANCE_THRESHOLD && result.language) {
      renderCode(text, result);
      return;
    }
  }

  // 2. Markdown 检测
  if (text.length > 0 && MD_PATTERN.test(text)) {
    renderMarkdown(text);
    return;
  }

  // 3. HTML 富文本渲染（仅 html 类型）
  if (clip.content_type === "html" && clip.html_content) {
    renderRichText(clip.html_content);
    return;
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
  code.innerHTML = result.value;
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
      _contentEl.appendChild(img);
    }
  } catch (e) {
    _contentEl.textContent = "图片加载失败";
    console.warn("预览图片加载失败:", e);
  }
}
