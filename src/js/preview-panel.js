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
import { detectionSample, limitForRender } from "./preview/large-text.js";
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
// content_hash → { language, relevance }
//
// 只缓存**结论**，不缓存高亮结果：高亮 HTML 比原文还大，200 条大条目能吃掉几百 MB，
// 而拿着已知语言重新高亮一遍只是一种语法的开销（自动检测是 21 种）。
const _hljsCache = new Map();

let _panelEl;
let _contentEl;
let _badgeEl;
let _metaEl;
let _renderers;
let _visible = false;
let _visibility;
let _currentClipId = null;
// 每轮 updatePreview 都递增。条目 id 不足以防竞态：同一 id 的重新渲染、延迟库加载
// 与旧图片 onload 都可能在 DOM 已经清空后回来，因此所有异步延续必须同时持有此代次。
let _renderGeneration = 0;

function _isCurrentRender(clipId, generation) {
  return _visible
    && _currentClipId === clipId
    && _renderGeneration === generation;
}

export function init({ onVisibilityChange } = {}) {
  _panelEl   = document.getElementById("preview-panel");
  _contentEl = document.getElementById("preview-content");
  _badgeEl   = document.getElementById("preview-type-badge");
  _metaEl    = document.getElementById("preview-meta");
  _visibility = createPanelVisibilityController({
    apply: (visible) => {
      _visible = visible;
      if (!visible) {
        _renderGeneration += 1;
        _currentClipId = null;
      }
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
    isCurrentRender: () => false,
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
  if (requested) {
    _renderGeneration += 1;
    _currentClipId = null;
  }
  try {
    await _visibility.request(requested);
  } catch (e) {
    console.warn("切换预览面板失败:", e);
  }
}

/** 清空预览内容，释放内存（窗口隐藏时调用） */
export function clearContent() {
  _renderGeneration += 1;
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
  const hadPendingUpdate = _debounceTimer !== null;
  clearTimeout(_debounceTimer);
  _debounceTimer = null;
  // 重复聚焦同一条目时保持正在进行的渲染；否则递增代次却被同 id 提前返回，
  // 会把图片/OCR 停在半成品状态。
  if (_visible && clip?.id === _currentClipId && !hadPendingUpdate) return;
  const generation = ++_renderGeneration;
  if (!_visible || !clip) {
    void _doUpdatePreview(clip, generation);
    return;
  }
  _debounceTimer = setTimeout(() => {
    _debounceTimer = null;
    void _doUpdatePreview(clip, generation);
  }, DEBOUNCE_MS);
}

async function _doUpdatePreview(clip, generation) {
  if (generation !== _renderGeneration) return;
  if (!_visible || !clip) {
    _contentEl.innerHTML = "";
    _contentEl.className = "preview-content";
    _badgeEl.textContent = "";
    _metaEl.textContent = "";
    _currentClipId = null;
    return;
  }

  _currentClipId = clip.id;
  const isCurrent = () => _isCurrentRender(clip.id, generation);

  const size = clip.byte_size;
  _metaEl.textContent = size > 1024
    ? `${(size / 1024).toFixed(1)} KB`
    : `${size} B`;

  _contentEl.innerHTML = "";
  _contentEl.className = "preview-content";

  if (clip.content_type === "image") {
    await _renderers.renderImage(clip, isCurrent);
    return;
  }

  // text 和 html 类型共享智能检测逻辑
  const text = clip.text_content || "";
  const trimmed = text.trim();

  // 同步可判的类型全部由 classify.js 那张有序表说话（顺序、badge、渲染器一处定义）
  const decision = classifyText(trimmed);
  if (decision) {
    if (decision.needsLibs) await ensureLibs();
    if (!isCurrent()) return;
    if (decision.renderer === "renderUrlCard") {
      _renderers.renderUrlCard(...decision.args, isCurrent);
    } else {
      _renderers[decision.renderer](...decision.args);
    }
    return;
  }

  // 延迟加载渲染库（首次调用时初始化 hljs/marked/DOMPurify）
  await ensureLibs();
  if (!isCurrent()) return;

  // 超大条目只画开头一段：几 MB 文本高亮出来的 DOM 有六位数节点，画完也滚不动。
  // 原文没动，复制/翻译/保存走的都是库里那份（见 preview/large-text.js）。
  const limited = limitForRender(text);

  // 1. Markdown 检测（优先，评分制，需多个特征）
  if (text.length > 0 && isMarkdown(text)) {
    _renderers.renderMarkdown(limited.body);
    _noteTruncation(limited);
    return;
  }

  // 2. 代码检测（text_content 纯文本，排除 xml 误判）
  if (text.length > 10) {
    const detected = _detectCodeLanguage(text, clip.content_hash);
    if (detected.relevance > CODE_RELEVANCE_THRESHOLD && detected.language && detected.language !== "xml") {
      // 已经知道语言，就按这一种语法高亮；`highlightAuto` 那 21 遍只用来做判断。
      const highlighted = hljs.highlight(limited.body, {
        language: detected.language,
        ignoreIllegals: true,
      });
      _renderers.renderCode(limited.body, {
        language: detected.language,
        relevance: detected.relevance,
        value: highlighted.value,
      });
      _noteTruncation(limited);
      return;
    }
  }

  // 3. HTML 富文本渲染（仅 html 类型，按需加载 html_content）
  if (clip.content_type === "html") {
    try {
      const detail = await getClipDetail(clip.id);
      if (!isCurrent()) return;
      if (detail.html_content) {
        // 富文本同样要限长：DOMPurify 要把整份 HTML 解析一遍。截断可能切在标签中间，
        // 而 DOMPurify 的解析器本来就负责补齐未闭合标签，不会漏出裸标签。
        const limitedHtml = limitForRender(detail.html_content);
        _renderers.renderRichText(limitedHtml.body);
        _noteTruncation(limitedHtml);
        return;
      }
    } catch (_) {
      if (!isCurrent()) return;
      // 详情读取失败时仅当前代次可以回退到纯文本。
    }
  }

  // 4. 纯文本
  if (!isCurrent()) return;
  _renderers.renderPlainText(limited.body);
  _noteTruncation(limited);
}

/**
 * 判定这段文本是什么语言。
 *
 * 检测只喂开头一段：`highlightAuto` 会拿注册的每种语法各跑一遍全文，一条几 MB 的
 * 日志能把 webview 按住好几秒，而"是什么语言"看开头就够了。结论按 content_hash 缓存
 * （内容不可变，缓存永不失效）。
 */
function _detectCodeLanguage(text, cacheKey) {
  const cached = cacheKey && _hljsCache.get(cacheKey);
  if (cached) return cached;
  const auto = hljs.highlightAuto(detectionSample(text));
  const detected = { language: auto.language, relevance: auto.relevance };
  if (cacheKey) {
    if (_hljsCache.size >= HLJS_CACHE_MAX) {
      _hljsCache.delete(_hljsCache.keys().next().value);
    }
    _hljsCache.set(cacheKey, detected);
  }
  return detected;
}

/** 截断了就说一声，否则用户会以为这条内容就这么长。 */
function _noteTruncation({ truncated, omitted }) {
  if (!truncated) return;
  const note = document.createElement("div");
  note.className = "preview-truncated";
  note.textContent = t("preview.truncated", { count: omitted.toLocaleString() });
  _contentEl.appendChild(note);
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
