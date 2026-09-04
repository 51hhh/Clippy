/**
 * preview/content-renderers.js — 富文本、URL、纯文本、图片与 OCR 预览
 */

import {
  getClipImage,
  ocrAvailable,
  ocrImage,
  getConfig,
  fetchUrlMeta,
  copyText,
} from "../api.ts";
import { t } from "../../i18n/i18n.js";

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

export function createContentRenderers({
  contentEl,
  badgeEl,
  metaEl,
  getLibraries,
  isCurrentRender = () => true,
}) {
  function renderMarkdown(text) {
    badgeEl.textContent = "MARKDOWN";
    contentEl.classList.add("preview-content--html");
    const rawHtml = getLibraries().marked.parse(text);
    contentEl.innerHTML = getLibraries().DOMPurify.sanitize(rawHtml, PURIFY_CONFIG);
  }

  function renderRichText(html) {
    badgeEl.textContent = "RICH TEXT";
    contentEl.classList.add("preview-content--html");
    contentEl.innerHTML = getLibraries().DOMPurify.sanitize(html, PURIFY_CONFIG);
  }

  function renderUrlCard(url, isCurrent = isCurrentRender) {
    badgeEl.textContent = "URL";
    contentEl.classList.add("preview-content--url");

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
    contentEl.appendChild(card);

    // 异步抓取 OG 元数据
    fetchUrlMeta(url).then(meta => {
      if (!isCurrent() || contentEl.querySelector(".url-card") !== card) return;
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
      if (!isCurrent()) return;
      loading.textContent = url;
    });
  }

  function renderPlainText(text) {
    badgeEl.textContent = "TEXT";
    contentEl.classList.add("preview-content--text");
    contentEl.textContent = text;
  }

  async function renderImage(clip, isCurrent = isCurrentRender) {
    badgeEl.textContent = "IMAGE";
    contentEl.classList.add("preview-content--image");
    try {
      const base64 = await getClipImage(clip.id);
      if (!isCurrent()) return;
      if (base64) {
        const img = document.createElement("img");
        img.src = `data:image/png;base64,${base64}`;
        img.alt = "clipboard image";
        img.onload = () => {
          if (!isCurrent()) return;
          metaEl.textContent = `${img.naturalWidth}×${img.naturalHeight} · ${
            clip.byte_size > 1024
              ? (clip.byte_size / 1024).toFixed(1) + " KB"
              : clip.byte_size + " B"
          }`;
        };
        contentEl.appendChild(img);

        // 自动 OCR：在图片下方显示可选择的识别文字
        const ocrArea = document.createElement("div");
        ocrArea.className = "preview-ocr-result";
        const ocrText = document.createElement("pre");
        ocrArea.appendChild(ocrText);
        contentEl.appendChild(ocrArea);

        // 检查 OCR 是否已启用
        try {
          const config = await getConfig();
          if (!isCurrent()) return;
          if (config.ocr_enabled === false) {
            ocrArea.classList.add("preview-ocr-result--hidden");
            return;
          }
        } catch (_) {
          if (!isCurrent()) return;
          // 读取配置失败则继续，保持预览 OCR 的默认行为。
        }

        // 先检查 OCR 是否可用
        const available = await ocrAvailable().catch(() => false);
        if (!isCurrent()) return;
        if (!available) {
          ocrText.textContent = t("action.ocrUnavailable");
          ocrArea.dataset.status = "unavailable";
          return;
        }

        ocrArea.dataset.status = "loading";
        ocrText.textContent = t("action.ocrProcessing");

        // 异步识别。每个异步边界都守住本次渲染代次，旧图片不得污染新预览，
        // 也不得在切换后继续发起 OCR 或复制识别结果。
        void ocrImage(clip.id).then(async (text) => {
          if (!isCurrent()) return;
          if (text && text.trim()) {
            // 检查配置：clipboard 模式直接复制，preview 模式显示文字
            try {
              const config = await getConfig();
              if (!isCurrent()) return;
              if (config.ocr_result_mode === "clipboard") {
                await copyText(text);
                if (!isCurrent()) return;
                ocrText.textContent = "✓ " + t("settings.ocr.clipboard");
                ocrArea.dataset.status = "done";
                return;
              }
            } catch (_) {
              if (!isCurrent()) return;
              // 读取配置失败则默认 preview 模式。
            }
            if (!isCurrent()) return;
            ocrText.textContent = text;
            ocrArea.dataset.status = "done";
          } else {
            ocrText.textContent = t("action.ocrEmpty");
            ocrArea.dataset.status = "empty";
          }
        }).catch(() => {
          if (!isCurrent()) return;
          ocrText.textContent = t("action.ocrFailed");
          ocrArea.dataset.status = "error";
        });
      }
    } catch (e) {
      if (!isCurrent()) return;
      contentEl.textContent = t("preview.imageLoadFailed");
      console.warn("预览图片加载失败:", e);
    }
  }

  return {
    renderMarkdown,
    renderRichText,
    renderUrlCard,
    renderPlainText,
    renderImage,
  };
}
