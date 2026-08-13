import { Copy, Edit3, Languages, Pin, Save, X } from "lucide-react";
import type { RefObject } from "react";
import type { CaptureAction, Rect } from "./types";
import { t } from "../shared/i18n";

export function OverlayToolbar({
  selection,
  viewportWidth,
  viewportHeight,
  busy,
  translationBusy,
  onAction,
  onTranslate,
  onCancel,
  translateButtonRef,
}: {
  selection: Rect;
  viewportWidth: number;
  viewportHeight: number;
  busy: boolean;
  translationBusy: boolean;
  onAction: (action: CaptureAction) => void;
  onTranslate: () => void;
  onCancel: () => void;
  translateButtonRef: RefObject<HTMLButtonElement>;
}) {
  const width = 231;
  const left = Math.max(8, Math.min(selection.x + selection.width - width, viewportWidth - width - 8));
  const preferredTop = selection.y + selection.height + 8;
  const top = preferredTop + 42 < viewportHeight ? preferredTop : Math.max(8, selection.y - 50);
  const buttons: Array<[CaptureAction, string, React.ReactNode]> = [
    ["copy", t("capture.copy"), <Copy size={16} />],
    ["save", t("capture.save"), <Save size={16} />],
    ["pin", t("capture.pin"), <Pin size={16} />],
    ["edit", t("capture.edit"), <Edit3 size={16} />],
  ];
  return (
    <div className="overlay-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {buttons.map(([action, label, icon]) => (
        <button key={action} type="button" title={label} aria-label={label} disabled={busy || translationBusy} onClick={() => onAction(action)}>
          {icon}
        </button>
      ))}
      <button
        ref={translateButtonRef}
        type="button"
        title={t("capture.translate")}
        aria-label={t("capture.translateSelection")}
        disabled={busy || translationBusy}
        onClick={onTranslate}
      >
        <Languages size={16} />
      </button>
      <span className="overlay-separator" />
      <button type="button" title={t("capture.cancel")} aria-label={t("capture.cancel")} disabled={busy} onClick={onCancel}>
        <X size={16} />
      </button>
    </div>
  );
}
