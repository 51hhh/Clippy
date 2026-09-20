import {
  ArrowLeft,
  Camera,
  Check,
  ClipboardCopy,
  Command,
  Languages,
  LoaderCircle,
  Play,
  Search,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ActionDescriptor,
  ActionHandle,
  ActionLauncherSettings,
  ActionReply,
} from "../../js/api.ts";
import { t } from "../shared/i18n";
import { launcherApi, type LauncherServices } from "./api";
import "../../styles/themes.css";
import "../../styles/base.css";
import "./launcher.css";

type DirectActionId = "capture.start" | "text.copy" | "text.translate";
type View = "list" | "form" | "running" | "result";

const DIRECT_ACTIONS: Record<DirectActionId, {
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

function isDirectAction(descriptor: ActionDescriptor): descriptor is ActionDescriptor & { id: DirectActionId } {
  return Object.hasOwn(DIRECT_ACTIONS, descriptor.id)
    && ["unit", "text", "translation_request"].includes(descriptor.input);
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
    action_clipboard_failed: "launcher.error.clipboard",
    launcher_window_failed: "launcher.error.window",
  };
  return t(specific[code] ?? "launcher.error.generic");
}

async function run(
  services: LauncherServices,
  id: DirectActionId,
  handle: ActionHandle,
): Promise<ActionReply<DirectActionId>> {
  switch (id) {
    case "capture.start": return services.run("capture.start", handle);
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
  const [attempt, setAttempt] = useState(0);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [selected, setSelected] = useState<(ActionDescriptor & { id: DirectActionId }) | null>(null);
  const [view, setView] = useState<View>("list");
  const [text, setText] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState(() => normalizedLanguage(settings.translationSourceLanguage, true));
  const [targetLanguage, setTargetLanguage] = useState(() => normalizedLanguage(settings.translationTargetLanguage, false));
  const [reply, setReply] = useState<ActionReply<DirectActionId> | null>(null);
  const [message, setMessage] = useState("");
  const [failure, setFailure] = useState("");
  const [cancelling, setCancelling] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const textRef = useRef<HTMLTextAreaElement>(null);
  const active = useRef<{ descriptor: ActionDescriptor & { id: DirectActionId }; handle: ActionHandle } | null>(null);
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
      if (current) setCatalog(descriptors.filter(isDirectAction));
    }).catch(() => {
      if (current) { setCatalog([]); setLoadFailed(true); }
    });
    return () => { current = false; };
  }, [services, attempt]);

  const visible = useMemo(() => {
    if (!catalog) return [];
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return catalog.filter(isDirectAction);
    return catalog.filter(isDirectAction).filter(descriptor => {
      const metadata = DIRECT_ACTIONS[descriptor.id];
      return [descriptor.id, t(metadata.title), t(metadata.description), metadata.keywords]
        .join(" ").toLocaleLowerCase().includes(needle);
    });
  }, [catalog, query]);

  useEffect(() => { setSelectedIndex(0); }, [query, catalog]);
  useEffect(() => {
    if (view === "list") searchRef.current?.focus();
    else if (view === "form" && selected?.id !== "capture.start") textRef.current?.focus();
  }, [view, selected]);

  function openAction(descriptor: ActionDescriptor & { id: DirectActionId }) {
    setSelected(descriptor); setView("form"); setFailure(""); setMessage(""); setReply(null);
  }
  function backToList() {
    if (view === "running") return;
    setSelected(null); setView("list"); setFailure(""); setMessage(""); setReply(null);
  }

  async function execute() {
    if (!selected || view === "running") return;
    if (selected.id !== "capture.start" && text.trim().length === 0) {
      setFailure(t("launcher.error.emptyText")); textRef.current?.focus(); return;
    }
    const ticket = ++operation.current;
    setView("running"); setFailure(""); setMessage(""); setReply(null); setCancelling(false);
    try {
      let handle: ActionHandle;
      if (selected.id === "capture.start") {
        handle = await services.prepare("capture.start", "launcher.capture", {});
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
    : "";

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
      {failure && <p className="launcher-error launcher-global-error" role="alert">{failure}</p>}
      <div className="launcher-results" role="listbox" aria-label={t("launcher.availableActions")}
        aria-activedescendant={visible[selectedIndex] ? `launcher-${visible[selectedIndex].id}` : undefined}>
        {catalog === null && <div className="launcher-state" role="status"><LoaderCircle className="is-spinning" />{t("launcher.loading")}</div>}
        {catalog !== null && visible.map((descriptor, index) => {
          const metadata = DIRECT_ACTIONS[descriptor.id];
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
        <span className="launcher-action-icon">{(() => { const Icon = DIRECT_ACTIONS[selected.id].icon; return <Icon size={20} />; })()}</span>
        <div><h1>{t(DIRECT_ACTIONS[selected.id].title)}</h1><p>{t(DIRECT_ACTIONS[selected.id].description)}</p></div>
      </div>

      {view === "form" && <form className="launcher-form" onSubmit={event => { event.preventDefault(); void execute(); }}>
        {selected.id === "capture.start" && <div className="launcher-callout"><Camera size={22} /><div><strong>{t("launcher.captureReady")}</strong><p>{t("launcher.captureHint")}</p></div></div>}
        {selected.id !== "capture.start" && <label className="launcher-field"><span>{t("launcher.text")}</span>
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
          <p>{t(selected.id === "text.copy" ? "launcher.copyCompleted" : "launcher.translationCompleted")}</p></div></div>
        {resultText && <pre tabIndex={0}>{resultText}</pre>}
        <button type="button" className="launcher-primary" onClick={backToList}>{t("launcher.done")}</button>
      </div>}
    </section>}
  </main>;
}
