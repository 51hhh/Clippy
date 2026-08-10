/**
 * preview/code-renderers.js — 代码、编码、JWT、哈希与颜色预览
 */

import { t } from "../../i18n/i18n.js";
import { normalizeColor, parseJwt } from "./detectors.js";

export function createCodeRenderers({ contentEl, badgeEl, getLibraries }) {
  function renderCode(text, result) {
    badgeEl.textContent = result.language.toUpperCase();
    contentEl.classList.add("preview-content--code");
    const pre = document.createElement("pre");
    const code = document.createElement("code");
    code.className = `hljs language-${result.language}`;
    code.innerHTML = getLibraries().DOMPurify.sanitize(result.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
    pre.appendChild(code);
    contentEl.appendChild(pre);
  }

  function renderJson(text) {
    badgeEl.textContent = "JSON";
    contentEl.classList.add("preview-content--code");
    let formatted;
    try { formatted = JSON.stringify(JSON.parse(text), null, 2); } catch { formatted = text; }
    const highlighted = getLibraries().hljs.highlight(formatted, { language: "json" });
    const pre = document.createElement("pre");
    const code = document.createElement("code");
    code.className = "hljs language-json";
    code.innerHTML = getLibraries().DOMPurify.sanitize(highlighted.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
    pre.appendChild(code);
    contentEl.appendChild(pre);
  }

  // ── 编码 / 哈希 / 加密 渲染函数 ────────────────────────────

  /** 可逆编码对照渲染 */
  function renderEncoded(result) {
    const LABELS = {
      "base64": "BASE64", "url-encoded": "URL ENCODED",
      "html-entity": "HTML ENTITY", "unicode": "UNICODE", "hex": "HEX",
    };
    badgeEl.textContent = LABELS[result.type] || result.type.toUpperCase();
    contentEl.classList.add("preview-content--encoded");

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

    contentEl.append(decodedSection, originalSection);
  }

  /** Base64 图片渲染 */
  function renderBase64Image(result) {
    badgeEl.textContent = "BASE64 → IMAGE";
    contentEl.classList.add("preview-content--image");
    const img = document.createElement("img");
    // 通过魔数推断格式
    const d = result.decoded;
    let mime = "image/png";
    if (d.startsWith("\xFF\xD8\xFF")) mime = "image/jpeg";
    else if (d.startsWith("GIF8")) mime = "image/gif";
    else if (d.startsWith("RIFF")) mime = "image/webp";
    img.src = `data:${mime};base64,${result.original.replace(/[\s\r\n]/g, "")}`;
    img.alt = "Base64 decoded image";
    contentEl.appendChild(img);
  }

  /** JWT 结构化渲染 */
  function renderJwt(text) {
    badgeEl.textContent = "JWT";
    contentEl.classList.add("preview-content--encoded");
    const { header, payload, signature } = parseJwt(text);

    // Header
    if (header) {
      const sec = _jwtSection("Header", JSON.stringify(header, null, 2));
      contentEl.appendChild(sec);
    }
    // Payload
    if (payload) {
      const sec = _jwtSection("Payload", JSON.stringify(payload, null, 2));
      contentEl.appendChild(sec);
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
    contentEl.appendChild(sigSec);
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
    const highlighted = getLibraries().hljs.highlight(jsonText, { language: "json" });
    code.innerHTML = getLibraries().DOMPurify.sanitize(highlighted.value, { ALLOWED_TAGS: ["span"], ALLOWED_ATTR: ["class"] });
    pre.appendChild(code);
    sec.appendChild(pre);
    return sec;
  }

  /** 哈希识别渲染 */
  function renderHash(text, hashType) {
    badgeEl.textContent = `HASH · ${hashType}`;
    contentEl.classList.add("preview-content--text");
    const mono = document.createElement("pre");
    mono.className = "encoded-box";
    mono.textContent = text;
    contentEl.appendChild(mono);
    const hint = document.createElement("div");
    hint.className = "encoded-hint";
    hint.textContent = t("preview.hashHint") || "Irreversible hash — cannot be decoded";
    contentEl.appendChild(hint);
  }

  /** 颜色值渲染：色块 + 格式信息 */
  function renderColor(text) {
    badgeEl.textContent = "COLOR";
    contentEl.classList.add("preview-content--encoded");

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

    contentEl.appendChild(wrapper);
  }

  return {
    renderCode,
    renderJson,
    renderEncoded,
    renderBase64Image,
    renderJwt,
    renderHash,
    renderColor,
  };
}
