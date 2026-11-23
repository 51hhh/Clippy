// 仅开发宿主提供合成图与服务，所有界面/坐标/绘制/关闭逻辑复用生产 App。
import React, { useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "../react/viewer/App";
import { init } from "../i18n/i18n.js";
import "./viewer.css";

if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const params = new URLSearchParams(location.search);
const boundedReviewSize = (name, min, max) => {
  const value = Number(params.get(name));
  return params.has(name) && Number.isFinite(value) ? Math.max(min, Math.min(max, Math.round(value))) : null;
};
const appWidth = boundedReviewSize("appWidth", 280, 1600), appHeight = boundedReviewSize("appHeight", 200, 1000);
const sampleText = "FIELD NOTES\nA walk by the river\n\nFollow the riverside path north. Record the time, signs and route.\n";
/** 合成图也是实际取色来源，不使用伪造的 HEX 或截图坐标。 */
function makeSample(long) {
  const canvas = document.createElement("canvas");
  canvas.width = long ? 640 : 1440;
  canvas.height = long ? 1800 : 960;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = "#f6f3e9"; ctx.fillRect(0, 0, canvas.width, canvas.height);
  if (long) {
    ctx.fillStyle = "#253a48"; ctx.font = "bold 34px sans-serif"; ctx.fillText("FIELD NOTES", 44, 72);
    ctx.font = "22px sans-serif"; ctx.fillText("Synthetic review sample", 44, 118);
    for (let row = 0; row < 25; row++) {
      ctx.fillStyle = row % 5 === 0 ? "#253a48" : "#9ca8aa";
      ctx.fillRect(44, 170 + row * 58, row % 5 === 0 ? 330 : 540 - row % 3 * 60, row % 5 === 0 ? 14 : 8);
    }
  } else {
    ctx.fillStyle = "#254940"; ctx.font = "bold 24px sans-serif"; ctx.fillText("FIELD NOTES  /  001", 88, 96);
    ctx.font = "52px Georgia, serif"; ctx.fillText("A walk by the river", 88, 179);
    ctx.fillStyle = "#769186"; ctx.font = "22px sans-serif"; ctx.fillText("Collect small details. Leave room for another day.", 90, 224);
    ctx.fillStyle = "#bdd4c6"; ctx.fillRect(88, 281, 1264, 432);
    ctx.fillStyle = "#829c76"; ctx.beginPath(); ctx.moveTo(88, 494); ctx.bezierCurveTo(330, 254, 624, 643, 1000, 367); ctx.bezierCurveTo(1170, 248, 1250, 336, 1352, 297); ctx.lineTo(1352, 713); ctx.lineTo(88, 713); ctx.fill();
    ctx.fillStyle = "#507867"; ctx.beginPath(); ctx.moveTo(88, 580); ctx.bezierCurveTo(427, 363, 706, 770, 1060, 538); ctx.lineTo(1352, 371); ctx.lineTo(1352, 713); ctx.lineTo(88, 713); ctx.fill();
    ctx.fillStyle = "#d5e8df"; ctx.beginPath(); ctx.moveTo(768, 281); ctx.bezierCurveTo(352, 400, 978, 486, 636, 713); ctx.lineTo(831, 713); ctx.bezierCurveTo(1049, 468, 565, 396, 845, 281); ctx.fill();
    ctx.fillStyle = "#254940"; ctx.font = "22px sans-serif"; ctx.fillText("09:40  Riverside path", 90, 787); ctx.fillText("10:15  The old station", 510, 787); ctx.fillText("11:00  North bridge", 944, 787);
    ctx.fillStyle = "#c9cbbd"; ctx.fillRect(90, 822, 1260, 1);
    for (const [i, color] of ["#254940", "#507867", "#829c76", "#bdd4c6", "#d5e8df"].entries()) { ctx.fillStyle = color; ctx.fillRect(90 + i * 42, 858, 28, 28); }
    ctx.fillStyle = "#769186"; ctx.font = "18px sans-serif"; ctx.fillText("A synthetic image for interaction review", 930, 879);
  }
  return { canvas, src: canvas.toDataURL("image/png"), width: canvas.width, height: canvas.height };
}


function ViewerReview() {
  const [language, setLanguage] = useState(params.get("lang") === "en" ? "en" : "zh-CN");
  const [theme, setTheme] = useState(params.get("theme") === "light" ? "light" : "dark");
  const [kind, setKind] = useState(params.get("sample") === "long" ? "long" : "landscape");
  const [closed, setClosed] = useState(false), [epoch, setEpoch] = useState(0), [activity, setActivity] = useState("");
  const [minimized, setMinimized] = useState(false), [fullscreen, setFullscreen] = useState(false);
  const notifyWindow = useRef(() => {});
  const zh = language === "zh-CN";
  init(language); document.documentElement.lang = language; document.documentElement.dataset.theme = theme;
  const services = useMemo(() => {
    const image = makeSample(kind === "long");
    const handle = { sessionId: `review-${kind}-${epoch}`, snapshotId: `snapshot-${kind}-${epoch}` };
    const payload = { handle, label: `image-viewer-${handle.sessionId}`, source: { clipId: null,
      contentHash: "a".repeat(64), width: image.width, height: image.height, byteLength: 16384,
      mediaType: "image/png", sensitive: false }, initialProject: null, limits: { canEdit: true, canScan: true, reason: null } };
    const reply = (request, value) => ({ ...request, value });
    const delay = async value => { await new Promise(resolve => setTimeout(resolve, 450)); return value; };
    const recognized = kind === "long" ? sampleText.repeat(14) : sampleText;
    const config = { language, theme, translation_target_language: "en", translation_source_language: "auto",
      translation_services: [{ provider: "libretranslate", enabled: true, endpoint: "https://example.test" }] };
    let nativeFullscreen = false;
    const windowListeners = new Set();
    notifyWindow.current = () => { for (const listener of windowListeners) listener(); };
    return {
      get: async () => payload, config: async () => config, imageUrl: () => image.src, ready: async () => {},
      onCloseRequested: async () => () => {}, close: async () => { setClosed(true); setFullscreen(false); },
      onWindowChanged: async callback => { windowListeners.add(callback); return () => windowListeners.delete(callback); },
      getFullscreen: async () => nativeFullscreen,
      setFullscreen: async (_, value) => {
        nativeFullscreen = value; setFullscreen(value);
        setActivity(value ? "DEMO Fullscreen · webpage only" : "DEMO Exit fullscreen · webpage only");
        queueMicrotask(() => { for (const listener of windowListeners) listener(); });
      },
      minimize: async () => { setMinimized(true); setActivity("DEMO Minimized · document retained · no native window change"); },
      startDrag: async () => { setActivity("DEMO Title drag · native window position is unchanged"); },
      recognize: async request => delay(reply(request, { width: image.width, height: image.height, text: recognized,
        lines: [], paragraphs: [], pipeline: { id: "review-only", engine: "tesseract", featureSchema: null,
          layoutExecuted: false, layoutReason: "synthetic-review" }, fallbackReason: "synthetic-review" })),
      scan: async request => delay(reply(request, { results: [{ format: "qr_code", text: "https://example.test/field-notes", points: [] }], limited: false })),
      translate: async (request, options) => delay(reply(request, { request_id: request.requestId,
        services: [{ provider: "libretranslate", status: "ok", detected_source_language: "en", target_language: options.targetLanguage || "en",
          translated_text: options.targetLanguage === "zh" ? "沿河笔记\n这是合成翻译结果，未发送网络请求。\n".repeat(kind === "long" ? 14 : 1) : recognized }] })),
      sample: async (request, x, y) => { const rgba = [...image.canvas.getContext("2d").getImageData(x, y, 1, 1).data];
        return reply(request, { x, y, rgba, hex: "#" + rgba.slice(0, 3).map(value => value.toString(16).padStart(2, "0")).join("").toUpperCase(), rgb: `rgb(${rgba.slice(0, 3).join(", ")})` }); },
      copyImage: async (request, document) => { setActivity(`DEMO Copy image · ${document?.annotations.length || 0} annotations · no clipboard write`); return reply(request, null); },
      save: async (request, mode, document) => { await delay(null); setActivity(`DEMO Save ${mode} · ${document?.annotations.length || 0} annotations · no file write`); return reply(request, { path: "DEMO / field-notes.png", clipboardWritten: mode === "editable", clipboardError: null }); },
      pin: async (request, document) => { setActivity(`DEMO Pin · ${document?.annotations.length || 0} annotations · no native window opened`); return reply(request, "review-pin"); },
      copyText: async (request, source, index) => { setActivity(`DEMO Copy ${source}[${index}] · no clipboard write`); return reply(request, null); },
    };
  }, [kind, epoch]);
  return <div className="viewer-review-shell">
    <div className="viewer-review-bar"><a href={`./index.html?lang=${language}&theme=${theme}`}>{zh ? "返回审阅页" : "Back to review"}</a><strong>{zh ? "真实生产组件 · 合成服务" : "Production component · synthetic services"}</strong>
      <span>{zh ? "OCR/扫码/翻译为演示；不联网、不写文件或剪贴板" : "Demo OCR/codes/translations; no network, files or clipboard"}</span>
      <div className="viewer-review-options"><select aria-label="Sample image" value={kind} onChange={event => { setKind(event.target.value); setClosed(false); setMinimized(false); setFullscreen(false); setActivity(""); }}><option value="landscape">{zh ? "横图" : "Landscape"}</option><option value="long">{zh ? "长图" : "Tall image"}</option></select>
        <select aria-label="Language" value={language} onChange={event => setLanguage(event.target.value)}><option value="zh-CN">中文</option><option value="en">English</option></select>
        <button type="button" onClick={() => setTheme(value => value === "dark" ? "light" : "dark")}>{zh ? "切换主题" : "Toggle theme"}</button></div>
    </div>
    <div className={`viewer-review-app${fullscreen ? " is-fullscreen" : ""}`} style={appWidth || appHeight ? { width: appWidth || undefined, height: appHeight || undefined, minHeight: 0, boxSizing: "content-box" } : undefined}>
      {closed ? <button type="button" onClick={() => { setEpoch(value => value + 1); setClosed(false); }}>{zh ? "已关闭 · 重新打开示例" : "Closed · reopen sample"}</button> : <>
        <div className={`viewer-review-document${minimized ? " is-minimized" : ""}`} inert={minimized ? "" : undefined}><App key={`${kind}-${epoch}`} services={services} /></div>
        {minimized && <div className="viewer-review-restore"><p>{zh ? "演示：窗口已最小化，文档已保留" : "Demo: minimized, document retained"}</p><button type="button" onClick={() => { setMinimized(false); notifyWindow.current(); }}>{zh ? "恢复查看器" : "Restore viewer"}</button></div>}
        {fullscreen && <span className="viewer-review-fullscreen-note">{zh ? "演示全屏 · Esc 返回" : "Demo fullscreen · Esc to return"}</span>}
      </>}
    </div>
    <p className="viewer-review-activity" role="status">{activity || (zh ? "缩放、平移、标注绘制与取色真实运行。原生后端由产品窗口提供。" : "Zoom, pan, annotation drawing and pixel sampling run. Native backend services are used by the product window.")}</p>
  </div>;
}
createRoot(document.getElementById("viewer-root")).render(<ViewerReview />);
