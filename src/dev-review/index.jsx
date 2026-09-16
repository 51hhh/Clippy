// 仅 Vite dev 入口：复用生产预览与翻译组件，所有服务均为本页合成数据。
import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { createContentRenderers } from "../js/preview/content-renderers.js";
import { createImageTranslationView } from "../js/preview/reveal-translation.js";
import { TranslationPanel } from "../react/main/TranslationPanel";
import { TranslationStore } from "../react/main/translationStore";
import { init as initLanguage } from "../i18n/i18n.js";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/components.css";
import "./review.css";

if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const delay = (milliseconds) => new Promise(resolve => setTimeout(resolve, milliseconds));
const samples = [
  ["long", "长图与长 OCR", "Tall image · long OCR"],
  ["empty", "没有识别文字", "No recognized text"],
  ["error", "识别失败与重试", "Recognition failure · retry"],
  ["loading", "识别中", "Recognizing"],
  ["disabled", "关闭自动识别", "Automatic OCR off"],
  ["image-error", "图片加载失败", "Image unavailable"],
  ["sensitive", "敏感内容保护", "Sensitive content"],
  ["text", "普通文本预览", "Plain text"],
];
function syntheticImage() {
  const canvas = document.createElement("canvas");
  canvas.width = 640; canvas.height = 1800;
  const context = canvas.getContext("2d");
  context.fillStyle = "#f8f7f3"; context.fillRect(0, 0, 640, 1800);
  context.fillStyle = "#253a48"; context.font = "bold 34px sans-serif";
  context.fillText("FIELD NOTES", 44, 72);
  context.font = "22px sans-serif"; context.fillText("Synthetic review sample", 44, 118);
  for (let row = 0; row < 25; row += 1) {
    context.fillStyle = row % 5 === 0 ? "#253a48" : "#9ca8aa";
    context.fillRect(44, 170 + row * 58, row % 5 === 0 ? 330 : 540 - row % 3 * 60, row % 5 === 0 ? 14 : 8);
  }
  return canvas.toDataURL("image/png").split(",")[1];
}
function Review() {
  const params = new URLSearchParams(location.search);
  const [language, setLanguage] = useState(params.get("lang") === "en" ? "en" : "zh-CN");
  const [theme, setTheme] = useState(params.get("theme") === "dark" ? "dark" : "light");
  const [scenario, setScenario] = useState("long");
  const [narrow, setNarrow] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [historyMode, setHistoryMode] = useState("all");
  const imageView = useMemo(createImageTranslationView, []);
  const viewerUrl = useRef("");
  viewerUrl.current = `./viewer.html?lang=${language}&theme=${theme}&sample=long`;
  const content = useRef(null); const badge = useRef(null); const meta = useRef(null);
  const zh = language === "zh-CN";
  initLanguage(language);
  const image = useMemo(syntheticImage, []);
  const text = useMemo(() => Array.from({ length: 14 }, (_, i) => zh
    ? `${String(i + 1).padStart(2, "0")}　旅行记录\n沿着河边步道向北行走，在旧车站附近停留。记下路牌、时间和路线，稍后整理成一份完整的旅行笔记。\n`
    : `${String(i + 1).padStart(2, "0")}  Field notes\nFollow the riverside path north and pause near the old station. Record the signs, time and route, then turn these observations into a complete travel journal.\n`).join("\n"), [zh]);
  const clip = useMemo(() => ({ id: samples.findIndex(item => item[0] === scenario) + 1,
    content_type: scenario === "text" ? "text" : "image", text_content: scenario === "text" ? "A quiet place to collect useful things.\n\nSelect text, copy it, or request a translation when needed." : null,
    html_content: null, image_data: null, content_hash: scenario, byte_size: 68352,
    is_sensitive: scenario === "sensitive", is_favorite: false, created_at: 1 }), [scenario]);
  const config = useMemo(() => ({ ocr_enabled: scenario !== "disabled", ocr_result_mode: "preview",
    translation_target_language: zh ? "en" : "zh", translation_services: [{ provider: "libretranslate", enabled: true,
      endpoint: "https://translate.example.test", model: "", region: "", project: "" }] }), [scenario, zh]);
  const store = useMemo(() => {
    const services = {
      copyText: async value => { setFeedback(`${zh ? "模拟复制" : "Simulated copy"}: ${value.length} characters`); },
      speakClip: async () => ({ mime_type: "audio/wav", audio_base64: "" }),
      speakText: async () => ({ mime_type: "audio/wav", audio_base64: "" }),
      translationHistory: async () => [],
      translateClip: async () => { await delay(600); return { request_id: 1, services: [{ status: "ok", provider: "libretranslate",
        translated_text: zh ? "Travel notes\n\n" + "Walk north along the riverside path and record the route, landmarks and time.\n\n".repeat(16) : "旅行笔记\n\n" + "沿着河边向北行走，记录路线、地标和时间。\n\n".repeat(16),
        detected_source_language: zh ? "zh" : "en", target_language: zh ? "en" : "zh" }] }; },
    };
    const result = new TranslationStore({ play: async () => { setFeedback(zh ? "模拟朗读：没有播放或网络请求" : "Simulated speech: no playback or network request"); }, stop: () => {} }, 0, services);
    result.setConfig(config); result.setClip(clip); result.setPanelVisible(true);
    return result;
  }, [clip, config, zh]);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = language;
  }, [theme, language]);
  useEffect(() => {
    let current = true; let attempts = 0;
    imageView.clear();
    content.current.replaceChildren(); content.current.className = "preview-content";
    content.current.closest(".preview-scroll").scrollTop = 0;
    setFeedback("");
    const renderer = createContentRenderers({ contentEl: content.current, badgeEl: badge.current, metaEl: meta.current,
      getLibraries: () => ({}), isCurrentRender: () => current, imageTranslation: imageView,
      imageActionLabel: zh ? "打开图片查看器" : "Open image viewer", services: {
        getClipImage: async () => { await delay(100); if (scenario === "image-error") throw new Error("Synthetic image failure"); return image; },
        ocrAvailable: async () => true,
        ocrImage: async () => { attempts += 1; if (scenario === "loading") return new Promise(() => {}); await delay(280);
          if (scenario === "error" && attempts === 1) throw new Error("Synthetic OCR failure");
          return scenario === "empty" ? "" : text; },
        getConfig: async () => config,
        copyText: async value => { setFeedback(`${zh ? "模拟复制" : "Simulated copy"}: ${value.length} characters`); },
        openImageViewer: async () => { location.href = viewerUrl.current; return "dev-review-viewer"; },
        fetchUrlMeta: async () => ({}),
      } });
    if (clip.content_type === "image") void renderer.renderImage(clip);
    else { renderer.renderPlainText(clip.text_content); meta.current.textContent = "Text"; }
    return () => { current = false; imageView.clear(); store.clear(); };
  }, [clip, config, image, imageView, scenario, store, text, zh]);
  return <main className="dev-review">
    <header className="review-header"><div><strong>Clippy</strong><span>{zh ? "开发审阅 · 合成数据" : "Development review · synthetic data"}</span></div>
      <nav><a href={`./index.html?lang=${language}`}>{zh ? "图片侧栏" : "Image sidebar"}</a><a href={`./pin.html?lang=${language}&mode=close`}>{zh ? "Pin 关闭" : "Pin close"}</a><a href={`./pin.html?lang=${language}&mode=save`}>{zh ? "Pin 保存" : "Pin save"}</a><a href={`./capture.html?lang=${language}`}>{zh ? "截图" : "Capture"}</a><a href={`./states.html?lang=${language}`}>{zh ? "更新与反馈" : "Updates & feedback"}</a></nav>
    </header>
    <p className="review-note">{zh ? "使用真实产品组件与主题。复制、识别、翻译和贴图为可控演示，不访问个人剪贴板、外部服务或系统窗口。" : "Actual product components and themes. Copy, recognition, translation and Pin are controlled demonstrations with no personal clipboard, external service or system window access."}</p>
    <div className="review-controls">
      <label>{zh ? "场景" : "Scenario"}<select value={scenario} onChange={event => setScenario(event.target.value)}>{samples.map(([id, cn, en]) => <option key={id} value={id}>{zh ? cn : en}</option>)}</select></label>
      <label>Language<select value={language} onChange={event => setLanguage(event.target.value)}><option value="zh-CN">中文</option><option value="en">English</option></select></label>
      <label>{zh ? "主题" : "Theme"}<select value={theme} onChange={event => setTheme(event.target.value)}><option value="light">{zh ? "浅色" : "Light"}</option><option value="dark">{zh ? "深色" : "Dark"}</option></select></label>
      <label className="review-checkbox"><input type="checkbox" checked={narrow} onChange={event => setNarrow(event.target.checked)} />{zh ? "窄侧栏" : "Narrow sidebar"}</label>
    </div>
    <div className={`review-window${narrow ? " review-window--narrow" : ""}`}>
      <aside className="review-history"><div className="review-search">{zh ? "搜索剪贴板历史…" : "Search clipboard history…"}</div><div className="review-tabs" role="tablist">{["all", "favorites"].map(mode => <button key={mode} role="tab" aria-selected={historyMode === mode} onClick={() => setHistoryMode(mode)}>{mode === "all" ? (zh ? "全部" : "All") : (zh ? "收藏" : "Favorites")}</button>)}</div><button className="review-history-row" onClick={() => setScenario("long")}><img src={`data:image/png;base64,${image}`} alt="" /><span>{zh ? "旅行笔记" : "Field notes"}<small>640 × 1800 · PNG</small></span></button><p>{zh ? "历史区为审阅背景；右侧预览、OCR 和翻译复用实际组件。点击图片审阅共享生产查看器组件。" : "History is review context. Preview, OCR and translation use actual components. Click the image to review the shared production viewer."}</p></aside>
      <section className={`preview-panel${clip.content_type === "image" ? " preview-panel--image" : ""}`}><div className="preview-header"><span ref={badge} className="preview-type-badge" /><span ref={meta} className="preview-meta" /></div><div className="preview-scroll"><div ref={content} className="preview-content" /><div className="translation-host"><TranslationPanel key={`${language}-${scenario}`} store={store} imageView={imageView} /></div></div></section>
    </div>
    <p className="review-feedback" role="status">{feedback || (zh ? "可测试：点击图片、文字复制、OCR 内翻译与返回、失败重试和键盘 Tab。" : "Try opening the image, Copy, OCR translation and Back, retry and keyboard Tab.")}</p>
  </main>;
}
createRoot(document.getElementById("review-root")).render(<Review />);
