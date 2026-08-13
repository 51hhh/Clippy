import { AlertTriangle, Copy, Languages, LoaderCircle, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { providerLabel } from "./translationState";
import type { CaptureTranslationState } from "./types";
import { t } from "../shared/i18n";

export function TranslationPopover({
  state,
  left,
  top,
  copyStatus,
  onCopy,
  onClose,
}: {
  state: CaptureTranslationState;
  left: number;
  top: number;
  copyStatus: "copied" | "failed" | null;
  onCopy: () => void;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLElement>(null);

  useEffect(() => {
    panelRef.current?.focus();
  }, [state.status]);

  return (
    <section
      ref={panelRef}
      className="translation-popover"
      style={{ left, top }}
      role="dialog"
      aria-labelledby="capture-translation-title"
      tabIndex={-1}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <header className="translation-header">
        <h2 id="capture-translation-title"><Languages size={16} />{t("capture.translation.title")}</h2>
        <button type="button" title={t("capture.translation.close")} aria-label={t("capture.translation.close")} onClick={onClose}>
          <X size={16} />
        </button>
      </header>

      {state.status === "loading" && (
        <div className="translation-progress" role="status" aria-live="polite">
          <LoaderCircle className="translation-spinner" size={18} />
          <div>
            <p>{t("capture.translation.ocrProgress")}</p>
            <span>{t("capture.translation.localPrivacy")}</span>
          </div>
        </div>
      )}

      {state.status === "error" && (
        <div className="translation-failure" role="alert">
          <AlertTriangle size={18} />
          <p>{state.message}</p>
        </div>
      )}

      {state.status === "result" && (
        <>
          <div className="translation-meta">
            <span>{providerLabel(state.result.provider)}</span>
            {state.result.detectedSourceLanguage && (
              <span>{t("capture.translation.detected", { language: state.result.detectedSourceLanguage })}</span>
            )}
          </div>
          <div className="translation-text">
            <h3>{t("capture.translation.original")}</h3>
            <pre>{state.result.sourceText}</pre>
            <h3>{t("capture.translation.result")}</h3>
            <pre>{state.result.translatedText}</pre>
          </div>
          <footer className="translation-footer">
            <span role="status" aria-live="polite">
              {copyStatus === "copied"
                ? t("capture.translation.copied")
                : copyStatus === "failed"
                  ? t("capture.translation.copyFailed")
                  : ""}
            </span>
            <button type="button" className="translation-copy" onClick={onCopy}>
              <Copy size={15} />{t("capture.translation.copy")}
            </button>
          </footer>
        </>
      )}
    </section>
  );
}
