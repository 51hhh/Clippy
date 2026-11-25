import { useLayoutEffect, useRef } from "react";
import { t } from "../shared/i18n";

type Props = {
  mode: "close" | "save";
  busy: boolean;
  error: string | null;
  onSave: () => void;
  onCancel: () => void;
  onAlternative: () => void;
};

/** 两个保存入口共享焦点边界；背景 inert 之外仍防程序化/过时事件穿透。 */
export function PinSaveDialog({ mode, busy, error, onSave, onCancel, onAlternative }: Props) {
  const dialog = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const returnFocus = useRef<HTMLElement | null>(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  useLayoutEffect(() => {
    cancel.current?.focus({ preventScroll: true });
    function containFocus(event: FocusEvent) {
      if (dialog.current && !dialog.current.contains(event.target as Node)) {
        const target = cancel.current?.disabled ? dialog.current : cancel.current;
        target?.focus({ preventScroll: true });
      }
    }
    document.addEventListener("focusin", containFocus);
    return () => {
      document.removeEventListener("focusin", containFocus);
      const previous = returnFocus.current;
      // layout cleanup 可能早于父级移除 inert；等提交完成，且没有新弹窗接管再恢复。
      queueMicrotask(() => {
        if (!document.querySelector("[data-pin-dialog]") && previous?.isConnected && !previous.closest("[inert]")) previous.focus({ preventScroll: true });
      });
    };
  }, []);
  useLayoutEffect(() => { if (busy) dialog.current?.focus({ preventScroll: true }); }, [busy]);
  useLayoutEffect(() => {
    // 重试失败后将正文定位到错误；不能让此前阅读隐私时的 scrollTop 把提示藏掉。
    if (error && body.current) body.current.scrollTop = 0;
  }, [error]);

  return <div className="pin-dialog-viewport" data-pin-dialog>
    <div className="pin-dialog-backdrop">
    <div ref={dialog} className="pin-close-prompt" role="dialog" aria-modal="true"
      aria-labelledby="pin-dialog-title" aria-describedby="pin-dialog-description pin-dialog-privacy"
      aria-busy={busy} tabIndex={-1}
      onKeyDown={(event) => {
        if (busy) { event.preventDefault(); event.stopPropagation(); return; }
        if (event.key === "Escape") {
          event.preventDefault(); event.stopPropagation(); onCancel();
        } else if (event.key === "Tab") {
          const focusable = [...(dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled), [data-pin-dialog-scroll]") ?? [])]
            .filter(element => !element.hasAttribute("data-pin-dialog-scroll") || element.scrollHeight > element.clientHeight);
          const index = focusable.indexOf(document.activeElement as HTMLElement);
          const next = index < 0 ? (event.shiftKey ? focusable.length - 1 : 0)
            : (index + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length;
          event.preventDefault();
          focusable[next]?.focus({ preventScroll: true });
        }
      }}>
      <h2 id="pin-dialog-title">{t(mode === "close" ? "pin.saveBeforeClose" : "pin.saveOptions")}</h2>
      <div ref={body} className="pin-dialog-body" data-pin-dialog-scroll tabIndex={busy ? -1 : 0}>
        {error && <p className="pin-dialog-error" role="alert">{error}</p>}
        <p id="pin-dialog-description">{t(mode === "close" ? "pin.closeDescription" : "pin.saveDescription")}</p>
        <p id="pin-dialog-privacy" className="pin-privacy-warning">{t("pin.editableContainsOriginal")}</p>
      </div>
      <div className="pin-close-actions">
        <button type="button" className="primary" disabled={busy} onClick={onSave}>
          {t(busy ? "pin.saving" : mode === "close" ? "pin.saveAndClose" : "pin.saveEditable")}
        </button>
        <button ref={cancel} type="button" disabled={busy} onClick={onCancel}>{t("pin.cancelClose")}</button>
      </div>
      <button type="button" className={`pin-dialog-alternative${mode === "close" ? " destructive" : ""}`}
        disabled={busy} onClick={onAlternative}>
        {t(mode === "close" ? "pin.discardAndClose" : "pin.exportFlat")}
      </button>
    </div>
    </div>
  </div>;
}
