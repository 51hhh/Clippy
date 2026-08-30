/**
 * preview-panel.js — 富文本预览面板
 *
 * 类型判定不在这里：同步可判的部分全在 `preview/classify.js` 的有序规则表里，
 * 这里只负责按表调渲染器，再兜住必须异步才能判的尾段：
 * 1. Markdown 正则匹配 → marked 渲染（badge: MARKDOWN）
 * 2. highlight.js 语言检测 → relevance > 5 → 代码高亮（badge: 语言名）
 * 3. html 类型 → 拉 html_content → DOMPurify 安全渲染（badge: RICH TEXT）
 * 4. 纯文本 → 原样显示（badge: TEXT）
 * image 类型走单独分支（badge: IMAGE）。
 *
 * 类型只显示在这块面板的 badge 上，列表行不显示——见 classify.js 的注释。
 *
 * 安全：剪贴板原始数据不可修改，所有处理仅在预览渲染层面。
 * 性能：hljs/marked/DOMPurify 延迟加载，首次打开预览面板时才初始化。
 */

import { getClipDetail, setPreviewVisible } from "./api.ts";
import { t } from "../i18n/i18n.js";
import * as detectors from "./preview/detectors.js";
import { classifyText } from "./preview/classify.js";
import { createPreviewRenderers } from "./preview/renderers.js";
import { createPanelVisibilityController } from "./panel-visibility.js";

const { isMarkdown } = detectors;

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

const STYLE_REMOVE_RE = /\b(background-color|background(?!-)|color|position|z-index|opacity)\s*:[^;]*(;|$)/gi;

// 代码检测阈值
const CODE_RELEVANCE_THRESHOLD = 5;
const HLJS_CACHE_MAX = 200;
const _hljsCache = new Map(); // content_hash → { language, relevance, value }

let _panelEl;
let _contentEl;
let _badgeEl;
let _metaEl;
let _renderers;
let _visible = false;
let _visibility;
let _currentClipId = null;

export function init({ onVisibilityChange } = {}) {
  _panelEl   = document.getElementById("preview-panel");
  _contentEl = document.getElementById("preview-content");
  _badgeEl   = document.getElementById("preview-type-badge");
  _metaEl    = document.getElementById("preview-meta");
  _visibility = createPanelVisibilityController({
    apply: (visible) => {
      _visible = visible;
      _panelEl.classList.toggle("hidden", !visible);
      // 面板显隐是翻译面板"是否值得查历史"的唯一依据（apply 可能重复调用，接收方需幂等）
      onVisibilityChange?.(visible);
    },
    persist: setPreviewVisible,
  });
  _renderers = createPreviewRenderers({
    contentEl: _contentEl,
    badgeEl: _badgeEl,
    metaEl: _metaEl,
    getLibraries: () => ({ hljs, DOMPurify, marked }),
    isCurrentClip: (clipId) => _currentClipId === clipId,
  });

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
  const requested = !_visible;
  if (requested) _currentClipId = null;
  try {
    await _visibility.request(requested);
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
  try {
    await _visibility.request(false);
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
    await _renderers.renderImage(clip);
    return;
  }

  // text 和 html 类型共享智能检测逻辑
  const text = clip.text_content || "";
  const trimmed = text.trim();

  // 同步可判的类型全部由 classify.js 那张有序表说话（顺序、badge、渲染器一处定义）
  const decision = classifyText(trimmed);
  if (decision) {
    if (decision.needsLibs) await ensureLibs();
    _renderers[decision.renderer](...decision.args);
    return;
  }

  // 延迟加载渲染库（首次调用时初始化 hljs/marked/DOMPurify）
  await ensureLibs();

  // 1. Markdown 检测（优先，评分制，需多个特征）
  if (text.length > 0 && isMarkdown(text)) {
    _renderers.renderMarkdown(text);
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
      _renderers.renderCode(text, result);
      return;
    }
  }

  // 3. HTML 富文本渲染（仅 html 类型，按需加载 html_content）
  if (clip.content_type === "html") {
    try {
      const detail = await getClipDetail(clip.id);
      if (_currentClipId !== clip.id) return; // 异步期间焦点已切换
      if (detail.html_content) {
        _renderers.renderRichText(detail.html_content);
        return;
      }
    } catch (_) { /* 回退到纯文本 */ }
  }

  // 4. 纯文本
  _renderers.renderPlainText(text);
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
