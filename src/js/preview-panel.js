/**
 * preview-panel.js — 富文本预览面板
 *
 * 支持三种渲染模式：
 * - text：纯文本 / 代码语法高亮（highlight.js 自动检测语言）
 * - html：DOMPurify 安全渲染富文本
 * - image：base64 PNG 预览
 *
 * Markdown 检测：text_content 首行匹配常见 Markdown 语法时自动切换 marked 渲染
 */

import { getClipImage, setPreviewVisible } from "./api.js";
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/core";
import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";

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
import css        from "highlight.js/lib/languages/css";
import yaml       from "highlight.js/lib/languages/yaml";
import markdown   from "highlight.js/lib/languages/markdown";
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
hljs.registerLanguage("css", css);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("ruby", ruby);
hljs.registerLanguage("php", php);
hljs.registerLanguage("swift", swift);
hljs.registerLanguage("kotlin", kotlin);
hljs.registerLanguage("lua", lua);

// highlight.js 样式通过 CSS 变量在 components.css 中定义，自动适配各主题

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
  // 安全：禁止脚本、iframe、object 等
  FORBID_TAGS: ["script","iframe","object","embed","form","input","textarea","button","select","option","link","meta","style"],
  FORBID_ATTR: ["onerror","onload","onclick","onmouseover","onfocus","onblur"],
};

// ── Marked 配置（Markdown → HTML，代码块用 highlight.js） ──
const marked = new Marked(
  markedHighlight({
    emptyLangClass: "hljs",
    langPrefix: "hljs language-",
    highlight(code, lang) {
      if (lang && hljs.getLanguage(lang)) {
        return hljs.highlight(code, { language: lang }).value;
      }
      return hljs.highlightAuto(code).value;
    },
  })
);

// Markdown 首行特征正则
const MD_PATTERN = /^(#{1,6}\s|[-*+]\s|\d+\.\s|>\s|```|---|\*\*|__|\[.+\]\(|!\[)/m;

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

/** 切换预览面板可见性 */
export async function toggle() {
  _visible = !_visible;
  _panelEl.classList.toggle("hidden", !_visible);
  if (_visible) _currentClipId = null; // 重置缓存以便重新渲染
  try {
    await setPreviewVisible(_visible);
  } catch (e) {
    console.warn("切换预览面板失败:", e);
  }
}

/** 获取当前可见状态 */
export function isVisible() {
  return _visible;
}

/** 隐藏预览面板 */
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

/**
 * 更新预览内容
 * @param {object|null} clip — ClipItem 对象或 null（清空）
 */
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

  const type = clip.content_type;

  // 元信息
  const size = clip.byte_size;
  _metaEl.textContent = size > 1024
    ? `${(size / 1024).toFixed(1)} KB`
    : `${size} B`;

  // 清除旧内容
  _contentEl.innerHTML = "";
  _contentEl.className = "preview-content";

  switch (type) {
    case "text":
      renderText(clip);
      break;
    case "html":
      renderHtml(clip);
      break;
    case "image":
      await renderImage(clip);
      break;
  }
}

/** 纯文本渲染：检测 Markdown / 代码语法自动切换 */
function renderText(clip) {
  const text = clip.text_content || "";

  // 检测 Markdown
  if (MD_PATTERN.test(text)) {
    _badgeEl.textContent = "MARKDOWN";
    _contentEl.classList.add("preview-content--html");
    const rawHtml = marked.parse(text);
    _contentEl.innerHTML = DOMPurify.sanitize(rawHtml, PURIFY_CONFIG);
    return;
  }

  // 尝试代码高亮（自动检测语言，置信度 > 阈值才启用）
  const result = hljs.highlightAuto(text);
  if (result.relevance > 5 && result.language) {
    _badgeEl.textContent = result.language.toUpperCase();
    _contentEl.classList.add("preview-content--code");
    const pre = document.createElement("pre");
    const code = document.createElement("code");
    code.className = `hljs language-${result.language}`;
    code.innerHTML = result.value;
    pre.appendChild(code);
    _contentEl.appendChild(pre);
    return;
  }

  // 普通纯文本
  _badgeEl.textContent = "TEXT";
  _contentEl.classList.add("preview-content--text");
  _contentEl.textContent = text;
}

/** HTML 安全渲染：DOMPurify 过滤后渲染富文本 */
function renderHtml(clip) {
  _badgeEl.textContent = "HTML";
  _contentEl.classList.add("preview-content--html");
  const html = clip.html_content || clip.text_content || "";
  _contentEl.innerHTML = DOMPurify.sanitize(html, PURIFY_CONFIG);
}

/** 图片预览 */
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
