// 仅开发审阅：复用实际 Capture App，合成冻结帧与输出服务。
import React, { useMemo } from "react";
import { createRoot } from "react-dom/client";
import { App } from "../react/capture-overlay/App";
import { init } from "../i18n/i18n.js";
import "../react/capture-overlay/overlay.css";


if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
document.getElementById("review-root").style.height = "100%";
const params = new URLSearchParams(location.search);
init(params.get("lang") === "en" ? "en" : "zh-CN");
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
function Review() {
  const setStatus = status => parent.postMessage({ type: "capture-review-status", status }, location.origin);
  const services = useMemo(() => {
    const width = innerWidth; const height = innerHeight;
    const frame = document.createElement("canvas"); frame.width = width; frame.height = height;
    const ctx = frame.getContext("2d");
    ctx.fillStyle = "#dde9e6"; ctx.fillRect(0, 0, width, height);
    ctx.fillStyle = "#153c3a"; ctx.font = "bold 28px sans-serif"; ctx.fillText("FIELD NOTES", 60, 76);
    ctx.font = "16px sans-serif"; ctx.fillText("Synthetic desktop · draw a selection, then try tools and sliders", 60, 112);
    ctx.fillStyle = "#fbfaf4"; ctx.fillRect(60, 145, Math.max(240, width - 120), Math.max(120, height - 200));
    ctx.fillStyle = "#8fa8a0";
    for (let i = 0; i < 7; i++) ctx.fillRect(90, 180 + 34 * i, Math.max(100, width - 220 - (i % 3) * 60), 9);
    const payload = { sessionId: "synthetic-capture", monitorId: 0, logicalX: 0, logicalY: 0,
      logicalWidth: width, logicalHeight: height, pixelWidth: width, pixelHeight: height, windows: [], probeHint: false };
    let handoff;
    const result = action => ({ action, path: action === "save" ? "/synthetic/capture.png" : null, pinLabel: action === "pin" ? "pin-synthetic" : null });
    return { windowLabel: () => "capture-overlay-review", api: {
      get: async () => payload,
      image: async () => { const image = new Image(); image.src = frame.toDataURL("image/png"); await image.decode(); return image; },
      frame: async () => ctx.getImageData(0, 0, width, height).data.buffer,
      ready: async () => {}, cancel: async () => setStatus("Cancel simulated · reload to reset"),
      closeUninitialized: async () => setStatus("Initialization close simulated"),
      commit: async action => {
        setStatus(`${action} simulated…`); await delay(800);
        if (params.get("result") !== "success") throw { message: "Synthetic output failure", outputPending: true, retryActions: ["copy", "save", "pin"] };
        setStatus(`${action} complete (simulated)`); return result(action);
      },
      retry: async action => { await delay(800); setStatus(`Retry ${action} complete (simulated)`); return result(action); },
      onHandoff: async callback => { handoff = callback; return () => { handoff = undefined; }; },
      openLongshot: async () => {
        await delay(600); handoff?.({ controllerLabel: "longshot-synthetic", sessionId: payload.sessionId, accepted: false });
        return { label: "longshot-synthetic" };
      },
      translate: async () => { throw new Error("Synthetic translation unavailable"); },
      copyText: async () => setStatus("Text copy simulated"),
    } };
  }, []);
  return <App services={services} />;
}
createRoot(document.getElementById("review-root")).render(<Review />);
