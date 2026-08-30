import {
  Check,
  Copy,
  Lock,
  LockOpen,
  Minus,
  Save,
  SlidersHorizontal,
  X,
  Plus,
} from "lucide-react";
import { t } from "../shared/i18n";

type Props = {
  scale: number;
  opacity: number;
  locked: boolean;
  canSave: boolean;
  copied: boolean;
  opacityOpen: boolean;
  onScale: (scale: number) => void;
  onOpacity: (opacity: number) => void;
  onToggleOpacity: () => void;
  onToggleLock: () => void;
  onCopy: () => void;
  onSave: () => void;
  onClose: () => void;
};

function ToolButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button type="button" className="pin-tool-button" aria-label={label} title={label} onClick={onClick}>
      {children}
    </button>
  );
}

export function PinToolbar(props: Props) {
  return (
    <div className="pin-controls" data-pin-controls onPointerDown={(event) => event.stopPropagation()}>
      <div className="pin-tools-vertical">
        <ToolButton label={t("pin.zoomIn")} onClick={() => props.onScale(props.scale + 0.1)}>
          <Plus size={16} />
        </ToolButton>
        <span className="pin-scale" aria-label={t("pin.scale", { percent: Math.round(props.scale * 100) })}>
          {Math.round(props.scale * 100)}
        </span>
        <ToolButton label={t("pin.zoomOut")} onClick={() => props.onScale(props.scale - 0.1)}>
          <Minus size={16} />
        </ToolButton>
        <span className="pin-tool-separator" />
        <ToolButton label={t(props.locked ? "pin.unlock" : "pin.lock")} onClick={props.onToggleLock}>
          {props.locked ? <Lock size={16} /> : <LockOpen size={16} />}
        </ToolButton>
        <ToolButton label={t("pin.opacity")} onClick={props.onToggleOpacity}>
          <SlidersHorizontal size={16} />
        </ToolButton>
        {props.canSave && (
          <ToolButton label={t("pin.save")} onClick={props.onSave}>
            <Save size={16} />
          </ToolButton>
        )}
        <ToolButton label={t("pin.copy")} onClick={props.onCopy}>
          {props.copied ? <Check size={16} /> : <Copy size={16} />}
        </ToolButton>
        <ToolButton label={t("pin.close")} onClick={props.onClose}>
          <X size={16} />
        </ToolButton>
      </div>
      {props.opacityOpen && (
        <div className="pin-opacity-popover">
          <input
            aria-label={t("pin.opacity")}
            type="range"
            min="15"
            max="100"
            value={Math.round(props.opacity * 100)}
            onChange={(event) => props.onOpacity(Number(event.target.value) / 100)}
          />
          <output>{Math.round(props.opacity * 100)}%</output>
        </div>
      )}
    </div>
  );
}
