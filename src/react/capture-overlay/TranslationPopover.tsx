import { AlertTriangle, Copy, Languages, LoaderCircle, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { providerLabel } from "./translationState";
import type { CaptureTranslationState } from "./types";

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
        <h2 id="capture-translation-title"><Languages size={16} />Translate selection</h2>
        <button type="button" title="Close translation" aria-label="Close translation" onClick={onClose}>
          <X size={16} />
        </button>
      </header>

      {state.status === "loading" && (
        <div className="translation-progress" role="status" aria-live="polite">
          <LoaderCircle className="translation-spinner" size={18} />
          <div>
            <p>Running local OCR...</p>
            <span>The image stays on this device.</span>
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
              <span>Detected {state.result.detectedSourceLanguage}</span>
            )}
          </div>
          <div className="translation-text">
            <h3>Original</h3>
            <pre>{state.result.sourceText}</pre>
            <h3>Translation</h3>
            <pre>{state.result.translatedText}</pre>
          </div>
          <footer className="translation-footer">
            <span role="status" aria-live="polite">
              {copyStatus === "copied" ? "Copied" : copyStatus === "failed" ? "Copy failed" : ""}
            </span>
            <button type="button" className="translation-copy" onClick={onCopy}>
              <Copy size={15} />Copy
            </button>
          </footer>
        </>
      )}
    </section>
  );
}
