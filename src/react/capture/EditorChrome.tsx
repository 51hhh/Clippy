import { Copy, Crop, Pin, Redo2, RotateCcw, Save, Undo2, X } from "lucide-react";

export function EditorHeader({
  busy,
  onRecapture,
  onClose,
}: {
  busy: boolean;
  onRecapture: () => void;
  onClose: () => void;
}) {
  return (
    <header className="capture-header">
      <div className="capture-title"><Crop size={18} /><span>Screenshot</span></div>
      <div className="capture-actions">
        <button className="capture-btn" type="button" onClick={onRecapture} disabled={busy}>
          <RotateCcw size={16} />Recapture
        </button>
        <button className="capture-icon-btn" type="button" onClick={onClose} title="Close" aria-label="Close">
          <X size={18} />
        </button>
      </div>
    </header>
  );
}

export function EditorFooter({
  busy,
  canUndo,
  canRedo,
  canExport,
  onUndo,
  onRedo,
  onReset,
  onExport,
}: {
  busy: boolean;
  canUndo: boolean;
  canRedo: boolean;
  canExport: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onReset: () => void;
  onExport: (action: "copy" | "save" | "pin") => void;
}) {
  return (
    <footer className="capture-footer">
      <ActionButton label="Undo" disabled={!canUndo || busy} onClick={onUndo}><Undo2 size={16} /></ActionButton>
      <ActionButton label="Redo" disabled={!canRedo || busy} onClick={onRedo}><Redo2 size={16} /></ActionButton>
      <ActionButton label="Reset" disabled={busy} onClick={onReset}><RotateCcw size={16} /></ActionButton>
      <div className="capture-footer-spacer" />
      <ActionButton label="Copy" primary disabled={!canExport} onClick={() => onExport("copy")}><Copy size={16} /></ActionButton>
      <ActionButton label="Save" disabled={!canExport} onClick={() => onExport("save")}><Save size={16} /></ActionButton>
      <ActionButton label="Pin" disabled={!canExport} onClick={() => onExport("pin")}><Pin size={16} /></ActionButton>
    </footer>
  );
}

function ActionButton({
  label,
  disabled,
  primary = false,
  onClick,
  children,
}: {
  label: string;
  disabled: boolean;
  primary?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button className={`capture-btn${primary ? " primary" : ""}`} type="button" onClick={onClick} disabled={disabled}>
      {children}{label}
    </button>
  );
}
