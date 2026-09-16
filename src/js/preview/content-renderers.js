/**
 * preview/content-renderers.js — 富文本、URL、纯文本、图片与 OCR 预览
 */

import {
  getClipImage,
  ocrAvailable,
  ocrImageResult,
  getConfig,
  fetchUrlMeta,
  copyText,
  openImageViewer,
} from "../api.ts";
import { t } from "../../i18n/i18n.js";
import { imageTranslationView, revealImageTranslation } from "./reveal-translation.js";

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

/** @type {(id: number) => Promise<import('../ipc-types.ts').StructuredOcr | string>} */
const recognizeImage = ocrImageResult;

export function createContentRenderers({
  contentEl,
  badgeEl,
  metaEl,
  getLibraries,
  isCurrentRender = () => true,
  imageTranslation = imageTranslationView,
  imageActionLabel = t("preview.openInViewer"),
  // 宿主服务可注入；默认仍通过唯一 api.ts 边界，审阅页不连接真实 IPC。
  services = { getClipImage, ocrAvailable, ocrImage: recognizeImage, getConfig, fetchUrlMeta, copyText, openImageViewer },
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
    services.fetchUrlMeta(url).then(meta => {
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
    const loading = document.createElement("p");
    loading.className = "preview-image-status";
    loading.setAttribute("role", "status");
    loading.textContent = t("preview.imageLoading");
    contentEl.appendChild(loading);
    try {
      const base64 = await services.getClipImage(clip.id);
      if (!isCurrent()) return;
      if (!base64) throw new Error("Image data unavailable");
      loading.remove();

      const imageCard = document.createElement("section");
      imageCard.className = "preview-image-card";
      const imageHeader = document.createElement("div");
      imageHeader.className = "preview-section-header";
      const title = document.createElement("h3");
      title.textContent = t("preview.imagePreview");
      const openButton = document.createElement("button");
      openButton.type = "button";
      openButton.className = "preview-image-open";
      openButton.setAttribute("aria-label", imageActionLabel);
      openButton.title = imageActionLabel;
      imageHeader.append(title);
      imageCard.appendChild(imageHeader);
      const img = document.createElement("img");
      img.alt = t("preview.imagePreview");
      img.onload = () => {
        if (!isCurrent() || !imageCard.contains(img)) return;
        metaEl.textContent = `${img.naturalWidth}×${img.naturalHeight} · ${
          clip.byte_size > 1024 ? (clip.byte_size / 1024).toFixed(1) + " KB" : clip.byte_size + " B"
        }`;
      };
      img.onerror = () => {
        if (!isCurrent()) return;
        openButton.disabled = true;
        img.remove();
        imageStatus.textContent = t("preview.imageLoadFailed");
      };
      img.src = `data:image/png;base64,${base64}`;
      openButton.appendChild(img);
      imageCard.appendChild(openButton);
      const imageStatus = document.createElement("p");
      imageStatus.className = "preview-image-status";
      imageStatus.setAttribute("role", "status");
      imageCard.appendChild(imageStatus);
      openButton.addEventListener("click", async () => {
        if (!isCurrent() || openButton.disabled) return;
        openButton.disabled = true;
        try {
          await services.openImageViewer(clip.id);
          if (isCurrent()) imageStatus.textContent = "";
        } catch (_) {
          if (isCurrent()) imageStatus.textContent = t("preview.viewerFailed");
        } finally {
          if (isCurrent()) openButton.disabled = false;
        }
      });
      contentEl.appendChild(imageCard);

      const ocrArea = document.createElement("section");
      ocrArea.className = "preview-ocr-result";
      const ocrHeader = document.createElement("div");
      ocrHeader.className = "preview-section-header";
      const ocrTitle = document.createElement("h3");
      ocrTitle.textContent = t("preview.ocrTitle");
      const copyButton = document.createElement("button");
      copyButton.type = "button";
      copyButton.className = "preview-ocr-copy";
      copyButton.textContent = t("action.copy");
      copyButton.disabled = true;
      const ocrActions = document.createElement("div");
      ocrActions.className = "preview-section-actions";
      const translateButton = document.createElement("button");
      translateButton.type = "button";
      translateButton.className = "preview-secondary-action preview-ocr-translate";
      translateButton.textContent = t("preview.translateOcrAction");
      translateButton.addEventListener("click", () => {
        if (isCurrent()) revealImageTranslation(contentEl.closest(".preview-panel") || contentEl, imageTranslation);
      });
      const backButton = document.createElement("button");
      backButton.type = "button";
      backButton.className = "preview-secondary-action preview-ocr-back";
      backButton.textContent = t("preview.backToOcr");
      backButton.hidden = true;
      ocrActions.append(translateButton, copyButton, backButton);
      ocrHeader.append(ocrTitle, ocrActions);
      const ocrText = document.createElement("pre");
      ocrText.tabIndex = 0;
      const feedback = document.createElement("p");
      feedback.className = "preview-ocr-feedback";
      feedback.setAttribute("role", "status");
      const retryButton = document.createElement("button");
      retryButton.type = "button";
      retryButton.className = "preview-secondary-action preview-ocr-retry";
      retryButton.textContent = t("preview.recognizeText");
      retryButton.hidden = true;
      const sourceView = document.createElement("div");
      sourceView.className = "preview-ocr-source";
      const provenance = document.createElement("div");
      provenance.className = "preview-ocr-provenance";
      provenance.hidden = true;
      sourceView.append(provenance, ocrText, feedback, retryButton);
      const translationSlot = document.createElement("div");
      translationSlot.className = "preview-ocr-translation";
      translationSlot.hidden = true;
      ocrArea.append(ocrHeader, sourceView, translationSlot);
      contentEl.appendChild(ocrArea);
      imageTranslation.bind({
        clipId: clip.id,
        container: translationSlot,
        isCurrent,
        onVisibilityChange(visible) {
          sourceView.hidden = visible;
          translationSlot.hidden = !visible;
          copyButton.hidden = visible;
          translateButton.hidden = visible;
          backButton.hidden = !visible;
          ocrTitle.textContent = t(visible ? "preview.translateOcr" : "preview.ocrTitle");
          ocrArea.dataset.view = visible ? "translation" : "source";
        },
      });
      backButton.addEventListener("click", () => {
        if (!isCurrent()) return;
        imageTranslation.hide(translationSlot);
        translateButton.focus({ preventScroll: true });
      });
      let recognizedText = "";
      const showOcr = (status, text) => {
        ocrArea.dataset.status = status;
        if (["loading", "disabled", "unavailable", "error"].includes(status)) provenance.hidden = true;
        ocrText.textContent = text;
        copyButton.disabled = status !== "done";
        retryButton.hidden = !["error", "empty", "disabled"].includes(status);
        retryButton.textContent = t(status === "error" ? "preview.retryOcr" : "preview.recognizeText");
      };
      copyButton.addEventListener("click", async () => {
        if (!isCurrent() || copyButton.disabled || !recognizedText) return;
        copyButton.disabled = true;
        try {
          await services.copyText(recognizedText);
          if (isCurrent()) feedback.textContent = t("codeScan.copied");
        } catch (_) {
          if (isCurrent()) feedback.textContent = t("codeScan.copyFailed");
        } finally {
          if (isCurrent()) copyButton.disabled = false;
        }
      });
      const runOcr = async () => {
        if (!isCurrent() || retryButton.disabled) return;
        retryButton.disabled = true;
        feedback.textContent = "";
        showOcr("loading", t("action.ocrProcessing"));
        try {
          const available = await services.ocrAvailable().catch(() => false);
          if (!isCurrent()) return;
          if (!available) {
            showOcr("unavailable", t("action.ocrUnavailable"));
            return;
          }
          const result = await services.ocrImage(clip.id);
          if (!isCurrent()) return;
          const text = typeof result === "string" ? result : result?.text;
          provenance.replaceChildren();
          // 注入式旧 String 服务没有管线信息，不猜测识别引擎；生产 API 始终返回结构化结果。
          if (result && typeof result !== "string") {
            const engine = document.createElement("span");
            engine.textContent = t(result.pipeline.engine === "ppocrv6+edgegnn" ? "viewer.enhancedOcr" : "viewer.tesseractOcr");
            provenance.appendChild(engine); provenance.hidden = false;
            if (result.fallbackReason) {
              const details = document.createElement("details");
              const summary = document.createElement("summary"); summary.textContent = t("preview.ocrFallbackReason");
              const reason = document.createElement("p"); reason.textContent = result.fallbackReason;
              details.append(summary, reason); provenance.appendChild(details);
            }
          }
          if (!text?.trim()) {
            showOcr("empty", t("action.ocrEmpty"));
            return;
          }
          // 原有自动复制模式仍生效，同时保留可读的识别正文与显式 Copy。
          const config = await services.getConfig().catch(() => ({}));
          if (!isCurrent()) return;
          recognizedText = text;
          showOcr("done", text);
          if (config.ocr_result_mode === "clipboard") {
            try {
              await services.copyText(text);
              if (isCurrent()) feedback.textContent = t("codeScan.copied");
            } catch (_) {
              if (isCurrent()) feedback.textContent = t("codeScan.copyFailed");
            }
          }
        } catch (_) {
          if (isCurrent()) showOcr("error", t("action.ocrFailed"));
        } finally {
          if (isCurrent()) retryButton.disabled = false;
        }
      };
      retryButton.addEventListener("click", () => void runOcr());
      showOcr("loading", t("action.ocrProcessing"));
      const config = await services.getConfig().catch(() => ({}));
      if (!isCurrent()) return;
      if (config.ocr_enabled === false) {
        showOcr("disabled", t("preview.ocrDisabled"));
        return;
      }
      // 保持 renderImage 的原有非阻塞 OCR 合同，代次由每个异步边界守护。
      void runOcr();
    } catch (error) {
      if (!isCurrent()) return;
      contentEl.replaceChildren();
      loading.textContent = t("preview.imageLoadFailed");
      contentEl.appendChild(loading);
      console.warn("预览图片加载失败:", error);
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
