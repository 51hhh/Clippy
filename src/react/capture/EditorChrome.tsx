import { Copy, Crop, Pin, Redo2, RotateCcw, Save, SaveAll, Undo2, X } from "lucide-react";
import { t } from "../shared/i18n";

/** 导出动作。saveAs 走系统另存为对话框，其余直接落到配置的目录。 */
export type ExportAction = "copy" | "save" | "saveAs" | "pin";

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
      <div className="capture-title"><Crop size={18} /><span>{t("capture.title")}</span></div>
      <div className="capture-actions">
        <button className="capture-btn" type="button" onClick={onRecapture} disabled={busy}>
          <RotateCcw size={16} />{t("capture.recapture")}
        </button>
        <button className="capture-icon-btn" type="button" onClick={onClose} title={t("capture.close")} aria-label={t("capture.close")}>
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
  onExport: (action: ExportAction) => void;
}) {
  return (
    <footer className="capture-footer">
      <ActionButton label={t("capture.undo")} disabled={!canUndo || busy} onClick={onUndo}><Undo2 size={16} /></ActionButton>
      <ActionButton label={t("capture.redo")} disabled={!canRedo || busy} onClick={onRedo}><Redo2 size={16} /></ActionButton>
      <ActionButton label={t("capture.reset")} disabled={busy} onClick={onReset}><RotateCcw size={16} /></ActionButton>
      <div className="capture-footer-spacer" />
      <ActionButton label={t("capture.copy")} primary disabled={!canExport} onClick={() => onExport("copy")}><Copy size={16} /></ActionButton>
      <ActionButton label={t("capture.save")} disabled={!canExport} onClick={() => onExport("save")}><Save size={16} /></ActionButton>
      <ActionButton label={t("capture.saveAs")} disabled={!canExport} onClick={() => onExport("saveAs")}><SaveAll size={16} /></ActionButton>
      <ActionButton label={t("capture.pin")} disabled={!canExport} onClick={() => onExport("pin")}><Pin size={16} /></ActionButton>
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
