// 仅开发审阅：实际 Pin App + 合成服务，任何按钮都不调用系统 API。
import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "../react/pin/App";
import { init, t } from "../i18n/i18n.js";
import "../styles/themes.css";
import "../react/pin/pin.css";
import "./pin-review.css";

if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const params = new URLSearchParams(location.search);
const language = params.get("lang") === "en" ? "en" : "zh-CN";
init(language);
document.documentElement.lang = language;
// Pin 独立窗口默认深色。审阅页显式加载现有主题 token 来验证弹窗兼容性。
document.documentElement.dataset.theme = params.get("theme") === "light" ? "light" : "dark";
const delay = (ms) => new Promise(resolve => setTimeout(resolve, ms));
function sampleImage() {
  const canvas = document.createElement("canvas"); canvas.width = 640; canvas.height = 420;
  const ctx = canvas.getContext("2d");
  ctx.fillStyle = "#f4f1e9"; ctx.fillRect(0, 0, 640, 420);
  ctx.fillStyle = "#173d43"; ctx.font = "bold 30px sans-serif"; ctx.fillText("A WALK BY THE RIVER", 36, 62);
  ctx.font = "16px sans-serif"; ctx.fillText("Synthetic image · review only", 36, 96);
  ctx.fillStyle = "#bfd7cf"; ctx.fillRect(36, 128, 568, 155);
  ctx.fillStyle = "#396b64"; ctx.beginPath(); ctx.moveTo(36, 267); ctx.bezierCurveTo(200, 130, 400, 325, 604, 167); ctx.lineTo(604, 283); ctx.lineTo(36, 283); ctx.fill();
  ctx.fillStyle = "#84958e"; for (let row = 0; row < 3; row++) ctx.fillRect(36, 317 + row * 26, 450 - row * 55, 8);
  return canvas.toDataURL("image/png");
}
async function until(predicate) {
  for (let attempt = 0; attempt < 120; attempt++) {
    const result = predicate(); if (result) return result;
    await delay(25);
  }
  throw new Error("Pin review did not become ready");
}
function Review() {
  const [status, setStatus] = useState("Synthetic services · no clipboard or file writes");
  const width = Math.max(240, Math.min(960, Number(params.get("width")) || 560));
  const height = Math.max(220, Math.min(900, Number(params.get("height")) || 450));
  const zoom = Math.max(.5, Math.min(2, Number(params.get("zoom")) || 1));
  const services = useMemo(() => {
    const image = sampleImage(); let nativeClose;
    const payload = { label: "pin-review", kind: "image", text: null, color: null,
      contentWidth: 640, contentHeight: 420, scale: Math.min((width - 68) / 640, (height - 72) / 420),
      opacity: 1, locked: false, above: false, canSave: true, position: null,
      deviceScale: 1, bufferScale: 1, initialProject: null };
    return { viewport: () => ({ width, height }), windowLabel: () => payload.label, startDragging: async () => setStatus("Synthetic window drag request"),
      requestClose: () => nativeClose?.(), api: {
        get: async () => payload, imageUrl: () => image,
        platform: async () => ({ capabilities: { always_on_top: { state: "available", reason: null } } }),
        ready: async () => {}, toolbarBounds: async () => ({ x: 0, y: 0, width, height }),
        update: async (_label, update) => Object.assign(payload, update),
        sourceImage: async () => image.split(",")[1],
        copy: async () => setStatus("Copy simulated"), copyCanvas: async () => setStatus("Canvas copy simulated"),
        save: async () => "/synthetic/pin.png",
        saveCanvas: async (_label, _png, _copy, mode, project) => {
          setStatus("Saving simulated (1.5 s)…"); await delay(1500);
          if (params.get("result") === "error") { setStatus("Simulated save failure"); throw new Error("Synthetic disk full"); }
          setStatus(`Saved ${mode} · ${project?.annotations?.length ?? 0} annotations (simulated)`);
          return { path: "/synthetic/pin.png", clipboardWritten: false, clipboardError: null };
        },
        close: async () => setStatus("Native close simulated · reload to reset"),
        onCloseRequested: async callback => { nativeClose = callback; return () => { nativeClose = undefined; }; },
        onSharpened: async () => () => {}, onAlreadyOpen: async () => () => {},
      } };
  }, [width, height]);
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const image = await until(() => document.querySelector(".pin-media img"));
      await until(() => image.complete && image.naturalWidth);
      if (cancelled) return;
      document.querySelector(`button[aria-label="${t("pin.canvasOpen")}"]`)?.click();
      // 标注通过实际交互层建立；仅合成事件临时跳过浏览器要求真实 pointer 的 capture。
      const canvas = await until(() => document.querySelector(".pin-canvas.editing"));
      await delay(80); if (cancelled) return;
      const bounds = canvas.getBoundingClientRect();
      const capture = canvas.setPointerCapture; const release = canvas.releasePointerCapture;
      canvas.setPointerCapture = () => {}; canvas.releasePointerCapture = () => {};
      try {
        for (const [type, x, y] of [["pointerdown", .15, .55], ["pointermove", .45, .6], ["pointerup", .65, .5]]) {
          canvas.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 21,
            clientX: bounds.left + bounds.width * x, clientY: bounds.top + bounds.height * y,
            button: 0, buttons: type === "pointerup" ? 0 : 1 }));
          await delay(25);
        }
      } finally { canvas.setPointerCapture = capture; canvas.releasePointerCapture = release; }
      if (cancelled || params.get("mode") === "edit") return;
      const undo = document.querySelector(`button[aria-label="${t("capture.undo")}"]`);
      undo?.focus({ preventScroll: true });
      if (params.get("mode") === "save") document.querySelector(`button[aria-label="${t("pin.save")}"]`)?.click();
      else services.requestClose();
    })().catch(error => setStatus(error.message));
    return () => { cancelled = true; };
  }, [services]);
  return <div className="pin-review-shell">
    <header><a href="./index.html">← Review</a><strong>Pin · actual App</strong><nav>
      {[240, 280, 320, 560].map(size => <a key={size} href={`?lang=${language}&mode=${params.get("mode") || "close"}&width=${size}&height=${height}`}>{size} px</a>)}
      <a href={`?lang=${language}&mode=${params.get("mode") || "close"}&width=${width}&height=240`}>240 px high</a>
      <a href={`?lang=${language}&mode=save&theme=light`}>Light · save</a>
      <a href="?lang=en&mode=close">English</a><a href="?lang=zh-CN&mode=close">中文</a>
      <a href={`?lang=${language}&mode=close&result=error`}>Save failure</a>
    </nav></header>
    <p role="status" className="pin-review-status">{status}</p>
    <div className="pin-review-stage" style={{ width, height, zoom }}><App services={services} /></div>
    <p className="pin-review-note">Width {width}px · height {height}px · scale {zoom}. Cancel restores focus to Undo. Tab stays in the dialog. Reload resets the synthetic document.</p>
  </div>;
}
createRoot(document.getElementById("review-root")).render(<Review />);
