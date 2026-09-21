// 仅开发宿主提供合成清单和本地 WebM；结果库组件、播放器与响应式样式均复用生产实现。
import React, { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { App } from "../react/recording-library/App.tsx";
import { init } from "../i18n/i18n.js";
import "../react/recording-library/recording-library.css";
import "./recordings-review.css";

if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const initialLanguage = new URLSearchParams(location.search).get("lang") === "en" ? "en" : "zh-CN";
let thumbnailFixture;

async function loadThumbnailFixture() {
  if (!thumbnailFixture) {
    thumbnailFixture = fetch(new URL("./recording-thumbnail-sample.png", import.meta.url))
      .then(response => response.arrayBuffer())
      .then(buffer => {
        const bytes = new Uint8Array(buffer);
        let binary = "";
        for (let offset = 0; offset < bytes.length; offset += 8192) {
          binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
        }
        return btoa(binary);
      });
  }
  return thumbnailFixture;
}

const complete = {
  sessionId: "review-complete",
  state: "complete",
  createdAtUnixMs: Date.UTC(2026, 8, 21, 8, 36),
  width: 1920,
  height: 1080,
  targetFpsNumerator: 30,
  targetFpsDenominator: 1,
  encoder: "vp9-prototype",
  container: "webm",
  includeCursor: true,
  droppedFrames: 3,
  durationMs: 82_400,
  frameCount: 2472,
  byteLength: 7_482_112,
  canMerge: false,
  canThumbnail: true,
  artifacts: [{
    artifactId: "final",
    displayName: "recording.webm",
    durationMs: 82_400,
    frameCount: 2472,
    byteLength: 7_482_112,
  }],
};

const interrupted = {
  ...complete,
  sessionId: "review-interrupted",
  state: "interrupted",
  createdAtUnixMs: Date.UTC(2026, 8, 20, 16, 18),
  width: 1440,
  height: 900,
  durationMs: 121_000,
  frameCount: 3630,
  byteLength: 10_821_632,
  canMerge: true,
  canThumbnail: true,
  artifacts: [0, 1].map(index => ({
    artifactId: `segment-${String(index).padStart(6, "0")}`,
    displayName: `segment-${String(index).padStart(6, "0")}.webm`,
    durationMs: index === 0 ? 60_000 : 61_000,
    frameCount: index === 0 ? 1800 : 1830,
    byteLength: index === 0 ? 5_312_512 : 5_509_120,
  })),
};

const diagnostic = {
  ...complete,
  sessionId: "review-diagnostic",
  state: "interrupted",
  createdAtUnixMs: Date.UTC(2026, 8, 19, 11, 2),
  encoder: "mjpeg",
  container: "avi",
  durationMs: 19_800,
  frameCount: 594,
  byteLength: 42_381_312,
  canMerge: false,
  canThumbnail: false,
  artifacts: [{
    artifactId: "segment-000000",
    displayName: "segment-000000.avi",
    durationMs: 19_800,
    frameCount: 594,
    byteLength: 42_381_312,
  }],
};

function RecordingReview() {
  const [language, setLanguage] = useState(initialLanguage);
  const [theme, setTheme] = useState("dark");
  const [width, setWidth] = useState(780);
  const [status, setStatus] = useState("");
  const [sessions, setSessions] = useState([complete, interrupted, diagnostic]);
  init(language);
  document.documentElement.lang = language;
  document.documentElement.dataset.theme = theme;
  const services = useMemo(() => ({
    ready: async () => {},
    list: async () => sessions,
    thumbnail: async sessionId => sessionId === "review-diagnostic"
      ? null
      : loadThumbnailFixture(),
    exportArtifact: async (sessionId, artifactId) => {
      setStatus(`DEMO export ${sessionId}/${artifactId} · no file write`);
      return true;
    },
    revealArtifact: async (sessionId, artifactId) => {
      setStatus(`DEMO reveal ${sessionId}/${artifactId} · no system window`);
    },
    deleteSession: async sessionId => {
      setSessions(value => value.filter(item => item.sessionId !== sessionId));
      setStatus(`DEMO delete ${sessionId} · synthetic list only`);
    },
    mergeSession: async sessionId => {
      setSessions(value => value.map(item => item.sessionId === sessionId ? {
        ...item,
        state: "complete",
        canMerge: false,
        artifacts: [{
          artifactId: "final",
          displayName: "recording.webm",
          durationMs: item.durationMs,
          frameCount: item.frameCount,
          byteLength: Math.max(1, Math.floor(item.byteLength * 0.98)),
        }],
      } : item));
      setStatus(`DEMO recover ${sessionId} · encoded packets remuxed without re-encoding`);
    },
    preparePlayback: async (sessionId, artifactId) => ({
      token: `media-${(sessionId + artifactId).length.toString(16).padStart(16, "0")}`,
      mimeType: "video/webm",
    }),
    releasePlayback: async token => setStatus(`DEMO release ${token}`),
    mediaUrl: () => new URL("./recording-sample.webm", import.meta.url).href,
    startDrag: async () => setStatus("DEMO title drag · no native window movement"),
    close: async () => setStatus("DEMO close · production window would be destroyed"),
  }), [sessions]);
  const zh = language === "zh-CN";
  return <main className="recordings-review-page">
    <nav className="recordings-review-bar">
      <a href="./index.html">{zh ? "返回审阅页" : "Back to review"}</a>
      <strong>{zh ? "录屏结果库 · 生产组件" : "Recording library · production component"}</strong>
      <button type="button" onClick={() => setWidth(value => value === 780 ? 420 : 780)}>{width}px</button>
      <button type="button" onClick={() => setTheme(value => value === "dark" ? "light" : "dark")}>{zh ? "切换主题" : "Toggle theme"}</button>
      <button type="button" onClick={() => setLanguage(value => value === "zh-CN" ? "en" : "zh-CN")}>{zh ? "English" : "中文"}</button>
    </nav>
    <section className="recordings-review-window" style={{ width }}>
      <App key={language} services={services} />
    </section>
    <p className="recordings-review-status" role="status">{status || (zh ? "播放使用本地合成 WebM；不读取录屏、文件或剪贴板。" : "Playback uses a local synthetic WebM; no recordings, files, or clipboard are read.")}</p>
  </main>;
}

createRoot(document.getElementById("recordings-review-root")).render(<RecordingReview />);
