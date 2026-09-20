import {
  ArrowLeft,
  Camera,
  Check,
  ClipboardCopy,
  Command,
  FileDown,
  Languages,
  LoaderCircle,
  Pin,
  Play,
  QrCode,
  Search,
  ScanText,
  ShieldAlert,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ActionDescriptor,
  ActionHandle,
  ActionId,
  ActionLauncherSettings,
  ActionReply,
  LauncherImageSource,
} from "../../js/api.ts";
import { t } from "../shared/i18n";
import { launcherApi, type LauncherServices } from "./api";
import "../../styles/themes.css";
import "../../styles/base.css";
import "./launcher.css";

type LauncherActionId = ActionId;
type View = "list" | "form" | "running" | "result";

const LAUNCHER_ACTIONS: Record<LauncherActionId, {
  icon: typeof Camera;
  title: string;
  description: string;
  keywords: string;
}> = {
  "capture.start": {
    icon: Camera,
    title: "launcher.action.capture",
    description: "launcher.action.captureDescription",
    keywords: "capture screenshot screen",
  },
  "image.ocr": {
    icon: ScanText,
    title: "launcher.action.ocr",
    description: "launcher.action.ocrDescription",
    keywords: "ocr recognize image text",
  },
  "image.pin": {
    icon: Pin,
    title: "launcher.action.pin",
    description: "launcher.action.pinDescription",
    keywords: "pin image window",
  },
  "image.save": {
    icon: FileDown,
    title: "launcher.action.save",
    description: "launcher.action.saveDescription",
    keywords: "save export image png",
  },
  "image.scan_codes": {
    icon: QrCode,
    title: "launcher.action.scan",
    description: "launcher.action.scanDescription",
    keywords: "scan qr barcode image",
  },
  "text.copy": {
    icon: ClipboardCopy,
    title: "launcher.action.copy",
    description: "launcher.action.copyDescription",
    keywords: "copy clipboard text",
  },
  "text.translate": {
    icon: Languages,
    title: "launcher.action.translate",
    description: "launcher.action.translateDescription",
    keywords: "translate language text",
  },
};

const LANGUAGE_KEYS: Array<[string, string]> = [
  ["auto", "settings.translation.languageAuto"],
  ["en", "settings.translation.languageEnglish"],
  ["zh", "settings.translation.languageChinese"],
  ["ja", "settings.translation.languageJapanese"],
  ["ko", "settings.translation.languageKorean"],
  ["es", "settings.translation.languageSpanish"],
  ["fr", "settings.translation.languageFrench"],
  ["de", "settings.translation.languageGerman"],
];

function isLauncherAction(descriptor: ActionDescriptor): descriptor is ActionDescriptor & { id: LauncherActionId } {
  return Object.hasOwn(LAUNCHER_ACTIONS, descriptor.id);
}

function canConstructInput(descriptor: ActionDescriptor, imageSource: LauncherImageSource | null): boolean {
  return descriptor.input !== "owned_image" || imageSource !== null;
}

function normalizedLanguage(value: string, allowAuto: boolean): string {
  return LANGUAGE_KEYS.some(([language]) => language === value && (allowAuto || language !== "auto"))
    ? value
    : allowAuto ? "auto" : "en";
}

function launcherErrorCode(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "code" in reason
    && typeof (reason as { code?: unknown }).code === "string") {
    return (reason as { code: string }).code;
  }
  const message = reason instanceof Error ? reason.message : typeof reason === "string" ? reason : "";
  const match = message.match(/(?:action|launcher|translation)_[a-z0-9_]+/i);
  return match?.[0] ?? "action_failed";
}

function errorMessage(code: string): string {
  const specific: Record<string, string> = {
    action_busy: "launcher.error.busy",
    action_capture_busy: "launcher.error.captureBusy",
    action_capture_session_busy: "launcher.error.captureBusy",
    action_capture_no_frames: "launcher.error.captureUnavailable",
    action_capture_screenshot_failed: "launcher.error.captureUnavailable",
    action_translation_empty_input: "launcher.error.emptyText",
    action_translation_input_too_large: "launcher.error.textTooLarge",
    action_translation_missing_api_key: "translation.error.missingApiKey",
    action_translation_incomplete_credentials: "translation.error.incompleteCredentials",
    action_translation_keyring_unavailable: "translation.error.keyringUnavailable",
    action_translation_no_service_enabled: "translation.error.noServiceEnabled",
    action_translation_timeout: "translation.error.timeout",
    action_translation_network: "translation.error.network",
    action_translation_rate_limited: "translation.error.rateLimited",
    action_translation_quota_exceeded: "translation.error.quotaExceeded",
    action_translation_sensitive_content: "translation.error.sensitive",
    action_clipboard_failed: "launcher.error.clipboard",
    action_image_source_unavailable: "launcher.error.imageSource",
    action_image_source_too_large: "launcher.error.imageTooLarge",
    action_ocr_failed: "launcher.error.ocr",
    action_code_scan_busy: "launcher.error.scanBusy",
    action_code_scan_failed: "launcher.error.scan",
    action_save_failed: "launcher.error.save",
    action_pin_failed: "launcher.error.pin",
    action_pin_uncertain: "launcher.error.pinUncertain",
    action_upstream_unavailable: "launcher.error.upstream",
    action_incompatible_output: "launcher.error.incompatible",
    launcher_image_source_failed: "launcher.error.imageSource",
    launcher_image_invalid: "launcher.error.imageInvalid",
    launcher_image_too_large: "launcher.error.imageTooLarge",
    launcher_window_failed: "launcher.error.window",
  };
  return t(specific[code] ?? "launcher.error.generic");
}

function completionMessage(id: LauncherActionId): string {
  const keys: Record<LauncherActionId, string> = {
    "capture.start": "launcher.captureCompleted",
    "image.ocr": "launcher.ocrCompleted",
    "image.pin": "launcher.pinCompleted",
    "image.save": "launcher.saveCompleted",
    "image.scan_codes": "launcher.scanCompleted",
    "text.copy": "launcher.copyCompleted",
    "text.translate": "launcher.translationCompleted",
  };
  return t(keys[id]);
}

async function run(
  services: LauncherServices,
  id: LauncherActionId,
  handle: ActionHandle,
): Promise<ActionReply<LauncherActionId>> {
  switch (id) {
    case "capture.start": return services.run("capture.start", handle);
    case "image.ocr": return services.run("image.ocr", handle);
    case "image.pin": return services.run("image.pin", handle);
    case "image.save": return services.run("image.save", handle);
    case "image.scan_codes": return services.run("image.scan_codes", handle);
    case "text.copy": return services.run("text.copy", handle);
    case "text.translate": return services.run("text.translate", handle);
  }
}

export function LauncherApp({
  settings,
  services = launcherApi,
}: {
  settings: ActionLauncherSettings;
  services?: LauncherServices;
}) {
  const [catalog, setCatalog] = useState<ActionDescriptor[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [imageSource, setImageSource] = useState<LauncherImageSource | null>(null);
  const [imageSourceFailed, setImageSourceFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [selected, setSelected] = useState<(ActionDescriptor & { id: LauncherActionId }) | null>(null);
  const [view, setView] = useState<View>("list");
  const [text, setText] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState(() => normalizedLanguage(settings.translationSourceLanguage, true));
  const [targetLanguage, setTargetLanguage] = useState(() => normalizedLanguage(settings.translationTargetLanguage, false));
  const [reply, setReply] = useState<ActionReply<LauncherActionId> | null>(null);
  const [message, setMessage] = useState("");
  const [failure, setFailure] = useState("");
  const [cancelling, setCancelling] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const textRef = useRef<HTMLTextAreaElement>(null);
  const active = useRef<{ descriptor: ActionDescriptor & { id: LauncherActionId }; handle: ActionHandle } | null>(null);
  const mounted = useRef(true);
  const closing = useRef(false);
  const operation = useRef(0);

  useEffect(() => () => { mounted.current = false; operation.current += 1; }, []);

  const close = useCallback(async () => {
    if (closing.current) return;
    const running = active.current;
    if (running && !running.descriptor.cancellable) {
      setFailure(t("launcher.error.runningCannotClose"));
      return;
    }
    closing.current = true;
    if (running) {
      try { await services.cancel(running.handle); } catch { /* 销毁会再次按窗口回收 */ }
    }
    try { await services.close(); }
    catch { if (mounted.current) { closing.current = false; setFailure(t("launcher.error.window")); } }
  }, [services]);
  const closeRef = useRef(close); closeRef.current = close;

  useEffect(() => {
    let current = true;
    let unlisten: (() => void) | undefined;
    void Promise.resolve().then(() => services.onCloseRequested(() => { void closeRef.current(); })).then(dispose => {
      if (!current) { dispose(); return; }
      unlisten = dispose;
      return services.ready();
    }).catch(() => {
      if (current) setFailure(t("launcher.error.window"));
    });
    return () => { current = false; unlisten?.(); };
  }, [services]);

  useEffect(() => {
    let current = true;
    setLoadFailed(false);
    void services.discover().then(descriptors => {
      if (current) setCatalog(descriptors.filter(isLauncherAction));
    }).catch(() => {
      if (current) { setCatalog([]); setLoadFailed(true); }
    });
    return () => { current = false; };
  }, [services, attempt]);

  useEffect(() => {
    let current = true;
    setImageSourceFailed(false);
    void Promise.resolve().then(() => services.imageSource()).then(source => {
      if (current) setImageSource(source);
    }).catch(() => {
      if (current) { setImageSource(null); setImageSourceFailed(true); }
    });
    return () => { current = false; };
  }, [services]);

  const visible = useMemo(() => {
    if (!catalog) return [];
    const needle = query.trim().toLocaleLowerCase();
    const available = catalog.filter(isLauncherAction).filter(descriptor => canConstructInput(descriptor, imageSource));
    if (!needle) return available;
    return available.filter(descriptor => {
      const metadata = LAUNCHER_ACTIONS[descriptor.id];
      return [descriptor.id, t(metadata.title), t(metadata.description), metadata.keywords]
        .join(" ").toLocaleLowerCase().includes(needle);
    });
  }, [catalog, query, imageSource]);

  useEffect(() => { setSelectedIndex(0); }, [query, catalog, imageSource]);
  useEffect(() => {
    if (view === "list") searchRef.current?.focus();
    else if (view === "form" && selected?.id !== "capture.start") textRef.current?.focus();
  }, [view, selected]);

  function openAction(descriptor: ActionDescriptor & { id: LauncherActionId }) {
    setSelected(descriptor); setView("form"); setFailure(""); setMessage(""); setReply(null);
  }
  function backToList() {
    if (view === "running") return;
    setSelected(null); setView("list"); setFailure(""); setMessage(""); setReply(null);
  }

  async function execute() {
    if (!selected || view === "running") return;
    if (["text", "translation_request"].includes(selected.input) && text.trim().length === 0) {
      setFailure(t("launcher.error.emptyText")); textRef.current?.focus(); return;
    }
    const ticket = ++operation.current;
    setView("running"); setFailure(""); setMessage(""); setReply(null); setCancelling(false);
    try {
      let handle: ActionHandle;
      if (selected.id === "capture.start") {
        handle = await services.prepare("capture.start", "launcher.capture", {});
      } else if (selected.id === "image.ocr") {
        if (!imageSource) throw { code: "action_image_source_unavailable" };
        handle = await services.prepare("image.ocr", "launcher.image.ocr", imageSource.reference);
      } else if (selected.id === "image.pin") {
        if (!imageSource) throw { code: "action_image_source_unavailable" };
        handle = await services.prepare("image.pin", "launcher.image.pin", imageSource.reference);
      } else if (selected.id === "image.save") {
        if (!imageSource) throw { code: "action_image_source_unavailable" };
        handle = await services.prepare("image.save", "launcher.image.save", imageSource.reference);
      } else if (selected.id === "image.scan_codes") {
        if (!imageSource) throw { code: "action_image_source_unavailable" };
        handle = await services.prepare("image.scan_codes", "launcher.image.scan", imageSource.reference);
      } else if (selected.id === "text.copy") {
        handle = await services.prepare("text.copy", "launcher.copy", { text });
      } else {
        handle = await services.prepare("text.translate", "launcher.translate", {
          text, sourceLanguage, targetLanguage,
        });
      }
      if (!mounted.current || ticket !== operation.current) return;
      active.current = { descriptor: selected, handle };
      const result = await run(services, selected.id, handle);
      if (!mounted.current || ticket !== operation.current) return;
      active.current = null;
      if (selected.id === "capture.start") { await services.close(); return; }
      setReply(result); setView("result");
    } catch (reason) {
      if (!mounted.current || ticket !== operation.current) return;
      active.current = null;
      const code = launcherErrorCode(reason);
      setView("form");
      if (code === "action_cancelled") setMessage(t("launcher.cancelled"));
      else setFailure(errorMessage(code));
    } finally {
      if (mounted.current && ticket === operation.current) setCancelling(false);
    }
  }

  async function executeComposition(actionId: "text.copy" | "text.translate") {
    if (!reply || !catalog || view === "running") return;
    const descriptor = catalog.filter(isLauncherAction).find(candidate => candidate.id === actionId);
    if (!descriptor) { setFailure(t("launcher.error.generic")); return; }
    const upstream = reply;
    const previousSelected = selected;
    const ticket = ++operation.current;
    setSelected(descriptor); setView("running"); setFailure(""); setMessage(""); setCancelling(false);
    try {
      const handle = actionId === "text.copy"
        ? await services.prepareComposed(upstream.handle, "text.copy", "launcher.compose.copy", {})
        : await services.prepareComposed(upstream.handle, "text.translate", "launcher.compose.translate", {
          sourceLanguage, targetLanguage,
        });
      if (!mounted.current || ticket !== operation.current) return;
      active.current = { descriptor, handle };
      const result = await run(services, actionId, handle);
      if (!mounted.current || ticket !== operation.current) return;
      active.current = null;
      setReply(result); setView("result");
    } catch (reason) {
      if (!mounted.current || ticket !== operation.current) return;
      active.current = null;
      setSelected(previousSelected); setReply(upstream); setView("result");
      const code = launcherErrorCode(reason);
      if (code === "action_cancelled") setMessage(t("launcher.cancelled"));
      else setFailure(errorMessage(code));
    } finally {
      if (mounted.current && ticket === operation.current) setCancelling(false);
    }
  }

  async function cancelRunning() {
    const running = active.current;
    if (!running || !running.descriptor.cancellable || cancelling) return;
    setCancelling(true); setFailure("");
    try { await services.cancel(running.handle); }
    catch (reason) {
      if (mounted.current && launcherErrorCode(reason) !== "action_cancelled") {
        setCancelling(false); setFailure(errorMessage(launcherErrorCode(reason)));
      }
    }
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.nativeEvent.isComposing) return;
    if (view === "list") {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        if (visible.length) setSelectedIndex(index => (index + (event.key === "ArrowDown" ? 1 : -1) + visible.length) % visible.length);
      } else if (event.key === "Enter" && visible[selectedIndex]) {
        event.preventDefault(); openAction(visible[selectedIndex]);
      } else if (event.key === "Escape") { event.preventDefault(); void close(); }
    } else if (view === "form") {
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") { event.preventDefault(); void execute(); }
      else if (event.key === "Escape") { event.preventDefault(); backToList(); }
    } else if (view === "result" && event.key === "Escape") {
      event.preventDefault(); backToList();
    }
  }

  const resultText = reply?.output.type === "translated_text"
    ? reply.output.value.translated_text
    : reply?.output.type === "recognized_text" ? reply.output.value.text : "";
  const scanResults = reply?.output.type === "detected_codes" ? reply.output.value.results : [];

  return <main className="launcher-window" onKeyDown={onKeyDown}>
    <header className="launcher-header" onPointerDown={event => {
      if (event.button !== 0 || (event.target as Element).closest("button,input,textarea,select,a")) return;
      event.preventDefault(); void services.startDrag();
    }}>
      <div className="launcher-brand"><Command size={18} /><strong>{t("launcher.title")}</strong><kbd>Ctrl K</kbd></div>
      <button type="button" className="launcher-close" aria-label={t("launcher.close")} title={t("launcher.close")}
        disabled={view === "running" && !selected?.cancellable} onClick={() => void close()}><X size={18} /></button>
    </header>

    {view === "list" && <section className="launcher-list-view">
      <label className="launcher-search">
        <Search size={18} aria-hidden="true" />
        <input ref={searchRef} value={query} onChange={event => setQuery(event.target.value)}
          placeholder={t("launcher.search")} aria-label={t("launcher.search")} autoComplete="off" spellCheck={false} />
        <kbd>Esc</kbd>
      </label>
      {imageSource && <div className="launcher-source-context" aria-label={t("launcher.imageContext")}>
        {imageSource.sensitive ? <ShieldAlert size={17} /> : <ScanText size={17} />}
        <span><strong>{t(imageSource.sensitive ? "launcher.sensitiveImage" : "launcher.currentImage")}</strong>
          <small>{imageSource.width} × {imageSource.height} · {(imageSource.byteLength / 1024).toFixed(1)} KB</small></span>
      </div>}
      {imageSourceFailed && <p className="launcher-error launcher-global-error" role="alert">{t("launcher.error.imageSource")}</p>}
      {failure && <p className="launcher-error launcher-global-error" role="alert">{failure}</p>}
      <div className="launcher-results" role="listbox" aria-label={t("launcher.availableActions")}
        aria-activedescendant={visible[selectedIndex] ? `launcher-${visible[selectedIndex].id}` : undefined}>
        {catalog === null && <div className="launcher-state" role="status"><LoaderCircle className="is-spinning" />{t("launcher.loading")}</div>}
        {catalog !== null && visible.map((descriptor, index) => {
          const metadata = LAUNCHER_ACTIONS[descriptor.id];
          const Icon = metadata.icon;
          return <button key={descriptor.id} id={`launcher-${descriptor.id}`} type="button" role="option"
            aria-selected={index === selectedIndex} className={`launcher-action${index === selectedIndex ? " is-selected" : ""}`}
            onPointerMove={() => setSelectedIndex(index)} onClick={() => openAction(descriptor)}>
            <span className="launcher-action-icon"><Icon size={19} /></span>
            <span><strong>{t(metadata.title)}</strong><small>{t(metadata.description)}</small></span>
            <kbd>↵</kbd>
          </button>;
        })}
        {catalog !== null && visible.length === 0 && <div className="launcher-state launcher-empty" role="status">
          <Search size={24} /><strong>{t(loadFailed ? "launcher.loadFailed" : "launcher.empty")}</strong>
          <span>{t(loadFailed ? "launcher.loadFailedHint" : "launcher.emptyHint")}</span>
          {loadFailed && <button type="button" onClick={() => setAttempt(value => value + 1)}>{t("launcher.retry")}</button>}
        </div>}
      </div>
      <footer className="launcher-hints"><span><kbd>↑</kbd><kbd>↓</kbd>{t("launcher.navigate")}</span><span><kbd>↵</kbd>{t("launcher.open")}</span></footer>
    </section>}

    {selected && view !== "list" && <section className="launcher-detail">
      <div className="launcher-detail-heading">
        <button type="button" className="launcher-back" aria-label={t("launcher.back")} disabled={view === "running"}
          onClick={backToList}><ArrowLeft size={18} /></button>
        <span className="launcher-action-icon">{(() => { const Icon = LAUNCHER_ACTIONS[selected.id].icon; return <Icon size={20} />; })()}</span>
        <div><h1>{t(LAUNCHER_ACTIONS[selected.id].title)}</h1><p>{t(LAUNCHER_ACTIONS[selected.id].description)}</p></div>
      </div>

      {view === "form" && <form className="launcher-form" onSubmit={event => { event.preventDefault(); void execute(); }}>
        {selected.id === "capture.start" && <div className="launcher-callout"><Camera size={22} /><div><strong>{t("launcher.captureReady")}</strong><p>{t("launcher.captureHint")}</p></div></div>}
        {selected.input === "owned_image" && imageSource && <div className="launcher-callout"><ScanText size={22} /><div>
          <strong>{t(imageSource.sensitive ? "launcher.sensitiveImage" : "launcher.currentImage")}</strong>
          <p>{imageSource.width} × {imageSource.height} · {(imageSource.byteLength / 1024).toFixed(1)} KB</p>
        </div></div>}
        {["text", "translation_request"].includes(selected.input) && <label className="launcher-field"><span>{t("launcher.text")}</span>
          <textarea ref={textRef} value={text} onChange={event => setText(event.target.value)} maxLength={262144}
            placeholder={t(selected.id === "text.translate" ? "launcher.translatePlaceholder" : "launcher.copyPlaceholder")} /></label>}
        {selected.id === "text.translate" && <div className="launcher-language-row">
          <label className="launcher-field"><span>{t("settings.translation.sourceLanguage")}</span><select value={sourceLanguage} onChange={event => setSourceLanguage(event.target.value)}>
            {LANGUAGE_KEYS.map(([value, key]) => <option key={value} value={value}>{t(key)}</option>)}
          </select></label>
          <label className="launcher-field"><span>{t("settings.translation.targetLanguage")}</span><select value={targetLanguage} onChange={event => setTargetLanguage(event.target.value)}>
            {LANGUAGE_KEYS.filter(([value]) => value !== "auto").map(([value, key]) => <option key={value} value={value}>{t(key)}</option>)}
          </select></label>
        </div>}
        {message && <p className="launcher-message" role="status">{message}</p>}
        {failure && <p className="launcher-error" role="alert">{failure}</p>}
        <div className="launcher-form-actions"><span>{selected.id === "capture.start" ? "" : t("launcher.runHint")}</span>
          <button type="submit" className="launcher-primary"><Play size={16} />{t("launcher.run")}</button></div>
      </form>}

      {view === "running" && <div className="launcher-running" role="status">
        <LoaderCircle className="is-spinning" size={34} /><strong>{t("launcher.running")}</strong><p>{t(`launcher.running.${selected.id}`)}</p>
        {selected.cancellable && <button type="button" disabled={!active.current || cancelling} onClick={() => void cancelRunning()}>
          {cancelling ? t("launcher.cancelling") : t("launcher.cancel")}
        </button>}
        {failure && <p className="launcher-error" role="alert">{failure}</p>}
      </div>}

      {view === "result" && <div className="launcher-result">
        <div className="launcher-result-title"><span><Check size={20} /></span><div><strong>{t("launcher.completed")}</strong>
          <p>{completionMessage(selected.id)}</p></div></div>
        {resultText && <pre tabIndex={0}>{resultText}</pre>}
        {scanResults.length > 0 && <div className="launcher-code-results">{scanResults.map((code, index) =>
          <div key={`${code.format}-${index}`}><strong>{code.format}</strong><pre tabIndex={0}>{code.text}</pre></div>)}</div>}
        {reply?.output.type === "detected_codes" && scanResults.length === 0
          && <p className="launcher-message" role="status">{t("launcher.noCodes")}</p>}
        {reply?.output.type === "saved_path" && <pre tabIndex={0}>{reply.output.value}</pre>}
        {failure && <p className="launcher-error" role="alert">{failure}</p>}
        {message && <p className="launcher-message" role="status">{message}</p>}
        <div className="launcher-result-actions">
          {reply?.output.type === "recognized_text" && resultText && <>
            {!imageSource?.sensitive && <button type="button" onClick={() => void executeComposition("text.translate")}><Languages size={16} />{t("launcher.composeTranslate")}</button>}
            <button type="button" onClick={() => void executeComposition("text.copy")}><ClipboardCopy size={16} />{t("launcher.composeCopy")}</button>
          </>}
          {reply?.output.type === "translated_text" && resultText
            && <button type="button" onClick={() => void executeComposition("text.copy")}><ClipboardCopy size={16} />{t("launcher.composeCopy")}</button>}
          <button type="button" className="launcher-primary" onClick={backToList}>{t("launcher.done")}</button>
        </div>
      </div>}
    </section>}
  </main>;
}
