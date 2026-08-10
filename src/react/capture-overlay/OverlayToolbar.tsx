import { Copy, Edit3, Pin, Save, X } from "lucide-react";
import type { CaptureAction, Rect } from "./types";

export function OverlayToolbar({
  selection,
  viewportWidth,
  viewportHeight,
  busy,
  onAction,
  onCancel,
}: {
  selection: Rect;
  viewportWidth: number;
  viewportHeight: number;
  busy: boolean;
  onAction: (action: CaptureAction) => void;
  onCancel: () => void;
}) {
  const width = 196;
  const left = Math.max(8, Math.min(selection.x + selection.width - width, viewportWidth - width - 8));
  const preferredTop = selection.y + selection.height + 8;
  const top = preferredTop + 42 < viewportHeight ? preferredTop : Math.max(8, selection.y - 50);
  const buttons: Array<[CaptureAction, string, React.ReactNode]> = [
    ["copy", "Copy", <Copy size={16} />],
    ["save", "Save", <Save size={16} />],
    ["pin", "Pin", <Pin size={16} />],
    ["edit", "Edit", <Edit3 size={16} />],
  ];
  return (
    <div className="overlay-toolbar" style={{ left, top }} onPointerDown={(event) => event.stopPropagation()}>
      {buttons.map(([action, label, icon]) => (
        <button key={action} type="button" title={label} aria-label={label} disabled={busy} onClick={() => onAction(action)}>
          {icon}
        </button>
      ))}
      <span className="overlay-separator" />
      <button type="button" title="Cancel" aria-label="Cancel" disabled={busy} onClick={onCancel}>
        <X size={16} />
      </button>
    </div>
  );
}
