import { useEffect, useRef, useState } from "react";
import { Image as ImageIcon, Maximize, Minimize, Minus, RotateCcw, X } from "lucide-react";
import type { ViewerSettings, PinCanvasSaveMode, ViewerPayload, ViewerTextSource } from "../../js/ipc-types";
import type { Tool } from "../annotation/types";
import { DEFAULT_COLOR, DEFAULT_STROKE } from "../capture-overlay/tools";
import { PinSaveDialog } from "../pin/PinSaveDialog";
import { t } from "../shared/i18n";
import { viewerApi, type ViewerServices } from "./api";
import { ViewerToolbar, type Panel } from "./ViewerToolbar";
import { ViewerTools, errorText } from "./ViewerTools";
import { useViewerCanvas } from "./useViewerCanvas";
import { useViewerDocument } from "./useViewerDocument";
import { useViewerTools, viewerError } from "./useViewerTools";
import { useViewerWindow } from "./useViewerWindow";
import "../../styles/themes.css";
import "../../styles/base.css";
import "../pin/pin.css";
import "./viewer.css";

export function App({ services = viewerApi }: { services?: ViewerServices }) {
  const [state, setState] = useState<{ payload: ViewerPayload; config: ViewerSettings | null } | null>(null);
  const [error, setError] = useState(false), [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let current = true; setError(false);
    void Promise.all([services.get(), services.config().catch(() => null)]).then(([payload, config]) => {
      if (current) setState({ payload, config });
    }).catch(() => { if (current) setError(true); });
    return () => { current = false; };
  }, [services, attempt]);
  if (!state) return <main className="viewer-startup" role="status">{t(error ? "viewer.loadFailed" : "preview.imageLoading")}{error && <button type="button" onClick={() => setAttempt(value => value + 1)}>{t("viewer.retry")}</button>}</main>;
  return <ViewerDocument key={`${state.payload.handle.sessionId}:${state.payload.handle.snapshotId}`} payload={state.payload} config={state.config} services={services} />;
}

/** 一个实例只持有一个 snapshot；源条目删除、其它窗口操作均不会替换文档。 */
export function ViewerDocument({ payload, config, services }: { payload: ViewerPayload; config: ViewerSettings | null; services: ViewerServices }) {
  const documentState = useViewerDocument(payload);
  const tools = useViewerTools(payload, services);
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [imageStatus, setImageStatus] = useState<"loading" | "ready" | "error">("loading");
  const [imageAttempt, setImageAttempt] = useState(0);
  const [listenerReady, setListenerReady] = useState(false);
  const [panel, setPanel] = useState<Panel | null>(null), [tool, setTool] = useState<Tool | "pan" | "color">("pan");
  const [color, setColor] = useState(DEFAULT_COLOR), [stroke, setStroke] = useState(DEFAULT_STROKE), [text, setText] = useState("");
  const [target, setTarget] = useState(config?.translation_target_language || "en");
  const [prompt, setPrompt] = useState<"save" | "close" | null>(null);
  const [busy, setBusy] = useState(false), busyRef = useRef(false), promptRef = useRef(prompt); promptRef.current = prompt;
  const [copying, setCopying] = useState(false), copyingRef = useRef(false);
  const [pinUncertain, setPinUncertain] = useState(false);
  const [feedback, setFeedback] = useState(""), [failure, setFailure] = useState<string | null>(null);
  const alive = useRef(true), closing = useRef(false);
  const windowControls = useViewerWindow(payload.handle, services, () => busyRef.current || !!promptRef.current || closing.current);
  const workspace = useRef<HTMLDivElement>(null);
  const canvas = useViewerCanvas({ image, source: payload.source, blocked: busy || prompt !== null,
    tool, color, stroke, text, annotations: documentState.annotations, adjustments: documentState.adjustments,
    selectedId: documentState.selectedId, select: documentState.setSelectedId, commit: documentState.commit,
    sample: (x, y) => { void tools.sample(x, y); }, onError: () => setFailure(t("viewer.renderFailed")),
  });
  const active = useRef({ documentState, canvas }); active.current = { documentState, canvas };
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => { workspace.current?.toggleAttribute("inert", busy || prompt !== null); }, [busy, prompt]);
  useEffect(() => {
    let current = true; setImageStatus("loading"); setImage(null);
    const source = new Image(); source.crossOrigin = "anonymous";
    source.onload = () => {
      if (!current) return;
      if (source.naturalWidth !== payload.source.width || source.naturalHeight !== payload.source.height) { setImageStatus("error"); return; }
      setImage(source); setImageStatus("ready");
    };
    source.onerror = () => { if (current) setImageStatus("error"); };
    source.src = services.imageUrl(payload);
    return () => { current = false; source.onload = null; source.onerror = null; };
  }, [payload, services, imageAttempt]);
  async function close() {
    if (closing.current) return;
    closing.current = true; active.current.canvas.cancel();
    try { await services.close(payload.handle); }
    catch (reason) { if (alive.current) { closing.current = false; setFailure(errorText(viewerError(reason))); } }
  }
  function requestClose() {
    if (busyRef.current || promptRef.current || closing.current) return;
    active.current.canvas.cancel();
    if (active.current.documentState.isDirty()) { promptRef.current = "close"; setPrompt("close"); setFailure(null); }
    else void close();
  }
  const requestCloseRef = useRef(requestClose); requestCloseRef.current = requestClose;
  useEffect(() => {
    let current = true; let unlisten: (() => void) | undefined;
    void services.onCloseRequested(() => requestCloseRef.current()).then(dispose => {
      if (!current) { dispose(); return; } unlisten = dispose; setListenerReady(true);
    }).catch(() => { if (current) { setFailure(t("viewer.closeListenerFailed")); setListenerReady(true); } });
    return () => { current = false; unlisten?.(); };
  }, [services]);
  useEffect(() => {
    if (!listenerReady || imageStatus === "loading") return;
    let current = true;
    void services.ready(payload.handle).catch(() => { if (current) setFailure(t("viewer.loadFailed")); });
    return () => { current = false; };
  }, [services, payload.handle, listenerReady, imageStatus]);
  async function output(kind: "copy" | "save" | "pin", mode: PinCanvasSaveMode = "editable") {
    if (busyRef.current || closing.current || !image || (kind === "pin" && pinUncertain)) return;
    busyRef.current = true; setBusy(true); setFailure(null); canvas.cancel();
    const captured = documentState.capture();
    const outputProject = captured.revision === 0 && payload.initialProject === null ? null : captured.project;
    const request = tools.authority.begin("output");
    try {
      if (kind === "save") {
        const reply = await services.save(request, mode, outputProject);
        if (!tools.authority.accepts("output", request, reply)) return;
        documentState.markSaved(captured.revision);
        if (reply.value.clipboardError || (mode === "editable" && !reply.value.clipboardWritten)) {
          setFeedback(t("viewer.saved", { path: reply.value.path }));
          setFailure(t("viewer.savedClipboardFailed"));
          setPrompt(null); promptRef.current = null;
        } else if (promptRef.current === "close" && documentState.version() === captured.revision) { setPrompt(null); promptRef.current = null; await close(); }
        else { setPrompt(null); promptRef.current = null; setFeedback(t("viewer.saved", { path: reply.value.path })); }
      } else {
        const reply = kind === "copy" ? await services.copyImage(request, outputProject) : await services.pin(request, outputProject);
        if (tools.authority.accepts("output", request, reply)) setFeedback(t(kind === "copy" ? "viewer.copied" : "viewer.pinned"));
      }
    } catch (reason) {
      if (tools.authority.accepts("output", request)) {
        const code = viewerError(reason);
        if (kind === "pin" && code === "pin_creation_uncertain") { setPinUncertain(true); setFailure(t("viewer.pinUncertain")); }
        else setFailure(errorText(code));
      }
    }
    finally { if (alive.current) { busyRef.current = false; setBusy(false); } }
  }
  async function copyResult(source: ViewerTextSource, index: number) {
    if (busyRef.current || copyingRef.current || promptRef.current || closing.current) return;
    copyingRef.current = true; setCopying(true); setFailure(null);
    const request = tools.authority.begin("copyText");
    try { const reply = await services.copyText(request, source, index); if (tools.authority.accepts("copyText", request, reply)) setFeedback(t("viewer.copied")); }
    catch (reason) { if (tools.authority.accepts("copyText", request)) setFailure(errorText(viewerError(reason))); }
    finally { if (alive.current) { copyingRef.current = false; setCopying(false); } }
  }
  function openPanel(next: Panel) {
    if (busyRef.current || promptRef.current) return;
    canvas.cancel(); setPanel(current => current === next ? null : next); setTool(next === "color" && panel !== next ? "color" : "pan");
  }
  function closePanel() { setPanel(null); if (tool === "color") setTool("pan"); canvas.canvasRef.current?.focus(); }
  const escapeFallback = useRef(() => {});
  escapeFallback.current = () => {
    if (promptRef.current || busyRef.current || closing.current) return;
    if (panel) closePanel(); else requestClose();
  };
  return <main className="viewer-window" data-snapshot-id={payload.handle.snapshotId}
    onKeyDownCapture={event => {
      if (event.key !== "F11") return;
      event.preventDefault(); event.stopPropagation();
      if (!event.nativeEvent.isComposing && !promptRef.current && !busyRef.current) { canvas.cancel(); windowControls.toggleFullscreen(); }
    }}
    onKeyDown={event => {
      if (event.defaultPrevented || event.nativeEvent.isComposing || promptRef.current || busyRef.current) return;
      const typing = (event.target as Element).closest("input,textarea,select,[contenteditable]");
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s" && !typing) { event.preventDefault(); promptRef.current = "save"; setPrompt("save"); setFailure(null); }
      else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z" && !typing) { event.preventDefault(); event.shiftKey ? documentState.redo() : documentState.undo(); }
      else if (event.key === "Escape") { event.preventDefault(); canvas.cancel(); windowControls.escape(() => escapeFallback.current()); }
    }}>
    <div className="viewer-workspace" ref={workspace}>
      <header className="viewer-header" onPointerDown={event => {
        if (event.defaultPrevented || event.button !== 0 || (event.target as Element).closest("button,input,select,textarea,a,[role=button]")) return;
        event.preventDefault(); canvas.cancel(); windowControls.startDrag();
      }}><ImageIcon size={20} /><div className="viewer-title"><h1>{t("viewer.title")}{documentState.dirty && <span aria-label={t("viewer.unsaved")}> ●</span>}</h1><p>{payload.source.width} × {payload.source.height} · PNG</p></div>
        <div className="viewer-window-controls">
          <button type="button" aria-label={t("viewer.minimize")} title={t("viewer.minimize")} disabled={busy || !!prompt || !!windowControls.pending} onClick={() => { canvas.cancel(); windowControls.minimize(); }}><Minus size={17} /></button>
          <button type="button" aria-label={t(windowControls.fullscreen ? "viewer.exitFullscreen" : "viewer.enterFullscreen")} title={t(windowControls.fullscreen ? "viewer.exitFullscreen" : "viewer.enterFullscreen")} aria-pressed={windowControls.fullscreen === true} disabled={busy || !!prompt || !!windowControls.pending} onClick={() => { canvas.cancel(); windowControls.toggleFullscreen(); }}>{windowControls.fullscreen ? <Minimize size={16} /> : <Maximize size={16} />}</button>
          <button type="button" className="viewer-close" aria-label={t("viewer.close")} title={t("viewer.close")} disabled={busy || !!prompt} onClick={requestClose}><X size={18} /></button>
        </div></header>
      <div className={`viewer-body${panel ? " has-panel" : ""}`}>
        <div ref={canvas.stageRef} className="viewer-canvas-stage">
          <canvas ref={canvas.canvasRef} className={`viewer-canvas${canvas.panning ? " is-dragging" : ""}`} tabIndex={0} aria-label={t("viewer.canvas")}
            data-scale={canvas.view.scale} data-pan-x={canvas.view.x} data-pan-y={canvas.view.y} style={{ cursor: tool === "color" ? "crosshair" : tool === "pan" ? "grab" : "crosshair" }} {...canvas.handlers} />
          {imageStatus !== "ready" && <div className="viewer-image-message" role="status">{t(imageStatus === "error" ? "viewer.loadFailed" : "preview.imageLoading")}{imageStatus === "error" && <button type="button" onClick={() => setImageAttempt(value => value + 1)}><RotateCcw size={14} />{t("viewer.retry")}</button>}</div>}
          <ViewerToolbar bounds={canvas.size} blocked={busy || prompt !== null || imageStatus !== "ready"} canEdit={payload.limits.canEdit} canScan={payload.limits.canScan} sensitive={tools.sensitive} pinUncertain={pinUncertain}
            tool={tool} setTool={next => { canvas.cancel(); setTool(next); }} panel={panel} openPanel={openPanel}
            color={color} setColor={setColor} stroke={stroke} setStroke={setStroke} text={text} setText={setText}
            canUndo={documentState.canUndo} canRedo={documentState.canRedo} selected={!!documentState.selectedId} undo={documentState.undo} redo={documentState.redo} remove={documentState.deleteSelected}
            percentage={Math.round(canvas.view.scale * canvas.dpr * 100)} fit={canvas.fit} actual={canvas.actual} zoom={canvas.zoom}
            copy={() => void output("copy")} save={() => { promptRef.current = "save"; setPrompt("save"); setFailure(null); }} pin={() => void output("pin")} />
        </div>
        {panel && <ViewerTools panel={panel} tools={tools} config={config} target={target} setTarget={setTarget} close={closePanel} blocked={busy || prompt !== null} copying={copying} copy={(source, index) => void copyResult(source, index)} />}
      </div>
      <footer className="viewer-status" role="status">{feedback || t("viewer.hint")}{payload.limits.reason && <span>{t("viewer.limited")}</span>}</footer>
      {failure && !prompt && <p className="viewer-error-text viewer-feedback-error" role="alert">{failure}</p>}
      {windowControls.failed && !prompt && <p className="viewer-error-text viewer-feedback-error" role="alert">{t("viewer.windowFailed")}</p>}
    </div>
    {prompt && <PinSaveDialog mode={prompt} busy={busy} error={failure}
      onSave={() => void output("save", "editable")}
      onCancel={() => { if (!busyRef.current) { promptRef.current = null; setPrompt(null); setFailure(null); } }}
      onAlternative={() => { if (busyRef.current) return; if (prompt === "close") void close(); else void output("save", "flat"); }} />}
  </main>;
}
