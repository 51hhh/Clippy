import { Copy, X } from "lucide-react";
import type { ViewerSettings, ViewerTextSource } from "../../js/ipc-types";
import { enabledTranslationServices, translationProviderMeta } from "../../js/translation-providers";
import { t } from "../shared/i18n";
import type { Panel } from "./ViewerToolbar";
import type { useViewerTools } from "./useViewerTools";

export function errorText(code: string | null): string {
  if (code === "busy") return t("viewer.errorBusy");
  if (code === "sensitive_content") return t("translation.sensitive");
  if (code === "clipboard_failed") return t("translation.copyFailed");
  if (code === "closed" || code === "stale_request") return t("viewer.errorClosed");
  return t("viewer.errorAction");
}
function fallbackText(reason: string): string {
  // 只映射稳定类别；未知后端内容不能把模型路径或内部进程错误带到界面。
  if (reason === "enhanced_configuration_invalid") return t("viewer.ocrFallbackConfiguration");
  if (reason === "enhanced_failed") return t("viewer.ocrFallbackFailed");
  return t("viewer.ocrFallbackUnknown");
}

type Props = {
  panel: Panel; tools: ReturnType<typeof useViewerTools>; config: ViewerSettings | null;
  target: string; setTarget: (value: string) => void; close: () => void;
  blocked: boolean; copying: boolean; copy: (source: ViewerTextSource, index: number) => void;
};

export function ViewerTools({ panel, tools, config, target, setTarget, close, blocked, copying, copy }: Props) {
  const state = panel === "ocr" ? tools.ocr : panel === "scan" ? tools.scan : panel === "translation" ? tools.translation : tools.color;
  const providers = enabledTranslationServices(config?.translation_services);
  const uncertainLines = tools.ocr.value?.lines.filter(line => !line.accepted && line.text.length > 0) || [];
  const copyButton = (source: ViewerTextSource, index: number, disabled = false) => <button type="button" className="viewer-copy-text" disabled={blocked || copying || disabled} onClick={() => copy(source, index)}><Copy size={14} />{t("action.copy")}</button>;
  return <aside id="viewer-tool-panel" className="viewer-panel" aria-label={t(`viewer.${panel}`)}>
    <header><h2>{t(`viewer.${panel}`)}</h2><button type="button" aria-label={t("viewer.closeTool")} disabled={blocked} onClick={close}><X size={17} /></button></header>
    <div className="viewer-panel-content">
      {panel === "ocr" && <>
        <p>{t("viewer.ocrHint")}</p>
        <button type="button" className="viewer-primary-button" disabled={blocked || state.status === "loading"} onClick={() => void tools.recognize()}>{t(state.status === "loading" ? "action.ocrProcessing" : "preview.recognizeText")}</button>
        {tools.ocr.value && <>
          <p className="viewer-engine" role="status">{t(tools.ocr.value.pipeline.engine === "ppocrv6+edgegnn" ? "viewer.enhancedOcr" : "viewer.tesseractOcr")}</p>
          {tools.ocr.value.fallbackReason && <p className="viewer-notice">{fallbackText(tools.ocr.value.fallbackReason)}</p>}
          {tools.ocr.value.pipeline.engine === "ppocrv6+edgegnn" && !tools.ocr.value.pipeline.layoutExecuted && <p className="viewer-notice">{t("viewer.layoutSkipped")}</p>}
          <div className="viewer-result-heading"><span>{t("viewer.result")}</span>{copyButton("ocr", 0, !tools.ocr.value.text.trim())}</div>
          <pre tabIndex={0}>{tools.ocr.value.text || t(uncertainLines.length ? "viewer.ocrNoAcceptedText" : "action.ocrEmpty")}</pre>
          {uncertainLines.length > 0 && <details className="viewer-ocr-details">
            <summary>{t("viewer.ocrUncertainDetails", { count: uncertainLines.length })}</summary>
            <p>{t("viewer.ocrUncertainHint")}</p>
            <ol>{uncertainLines.map(line => <li key={line.id}>
              <span>{t("viewer.ocrConfidence", { value: Math.round(line.confidence * 100) })}</span>
              <pre tabIndex={0}>{line.text}</pre>
            </li>)}</ol>
          </details>}
        </>}
      </>}
      {panel === "scan" && <>
        <p>{t("viewer.scanHint")}</p><button type="button" className="viewer-primary-button" disabled={blocked || state.status === "loading"} onClick={() => void tools.detect()}>{t(state.status === "loading" ? "codeScan.scanning" : "viewer.scan")}</button>
        {tools.scan.value?.limited && <p className="viewer-notice">{t("codeScan.limited")}</p>}
        {tools.scan.value && tools.scan.value.results.length === 0 && <p>{t("codeScan.empty")}</p>}
        {tools.scan.value?.results.map((code, index) => <section className="viewer-code-result" key={index}><div className="viewer-result-heading"><span>{code.format === "qr_code" ? "QR Code" : "Code 39"}</span>{copyButton("code", index, !code.text)}</div><pre tabIndex={0}>{code.text}</pre></section>)}
      </>}
      {panel === "translation" && <>
        <p>{t("translation.privacy")}</p>
        <label className="viewer-field">{t("viewer.target")}<select value={target} disabled={blocked || state.status === "loading"} onChange={event => setTarget(event.target.value)}>{["en", "zh", "ja", "ko", "es", "fr", "de"].map(language => <option key={language} value={language}>{({ en: "English", zh: "中文", ja: "日本語", ko: "한국어", es: "Español", fr: "Français", de: "Deutsch" })[language]}</option>)}</select></label>
        <ul className="viewer-services">{providers.map(service => { const meta = translationProviderMeta(service.provider); return <li key={service.provider}><strong>{t(meta.nameKey)}</strong><span>{service.endpoint || meta.defaultEndpoint}</span></li>; })}</ul>
        {providers.length === 0 && <p>{t("translation.error.noServiceEnabled")}</p>}
        {tools.sensitive && <p role="status" className="viewer-notice">{t("translation.sensitive")}</p>}
        <button type="button" className="viewer-primary-button" disabled={blocked || state.status === "loading" || tools.sensitive || providers.length === 0} onClick={() => void tools.translate({ sourceLanguage: config?.translation_source_language || "auto", targetLanguage: target })}>{t(state.status === "loading" ? "translation.ocrTranslating" : "translation.ocrAndTranslate")}</button>
        {tools.translation.value?.services.map((result, index) => <section className="viewer-translation-result" key={result.provider}><div className="viewer-result-heading"><strong>{t(translationProviderMeta(result.provider).nameKey)}</strong>{result.status === "ok" && copyButton("translation", index, !result.translated_text)}</div>
          {result.status === "ok" ? <><span className="viewer-result-target">{t("translation.target", { language: result.target_language })}</span><pre tabIndex={0}>{result.translated_text}</pre></> : <p role="alert">{errorText(result.code)}</p>}</section>)}
        <p className="viewer-notice">{t("viewer.layoutUnsupported")}</p>
      </>}
      {panel === "color" && <>
        <p>{t("viewer.colorHint")}</p>
        {tools.color.value && <><div className="viewer-color-preview" style={{ backgroundColor: `rgba(${tools.color.value.rgba[0]},${tools.color.value.rgba[1]},${tools.color.value.rgba[2]},${tools.color.value.rgba[3] / 255})` }} />
          <dl className="viewer-color-details"><div><dt>HEX</dt><dd>{tools.color.value.hex}</dd></div><div><dt>RGBA</dt><dd>{tools.color.value.rgba.join(", ")}</dd></div><div><dt>{t("viewer.pixel")}</dt><dd>{tools.color.value.x}, {tools.color.value.y}</dd></div></dl>{copyButton("color", 0)}</>}
      </>}
      {state.status === "loading" && <p role="status">{t("viewer.working")}</p>}
      {state.status === "error" && <p role="alert" className="viewer-error-text">{errorText(state.error)}</p>}
    </div>
  </aside>;
}
