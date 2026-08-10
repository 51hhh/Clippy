import {
  Check,
  Copy,
  Edit3,
  Lock,
  LockOpen,
  Minus,
  Save,
  SlidersHorizontal,
  X,
  Plus,
} from "lucide-react";

type Props = {
  scale: number;
  opacity: number;
  locked: boolean;
  canSave: boolean;
  canEdit: boolean;
  copied: boolean;
  opacityOpen: boolean;
  onScale: (scale: number) => void;
  onOpacity: (opacity: number) => void;
  onToggleOpacity: () => void;
  onToggleLock: () => void;
  onCopy: () => void;
  onSave: () => void;
  onEdit: () => void;
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
        <ToolButton label="Zoom in" onClick={() => props.onScale(props.scale + 0.1)}>
          <Plus size={16} />
        </ToolButton>
        <span className="pin-scale" aria-label={`Scale ${Math.round(props.scale * 100)} percent`}>
          {Math.round(props.scale * 100)}
        </span>
        <ToolButton label="Zoom out" onClick={() => props.onScale(props.scale - 0.1)}>
          <Minus size={16} />
        </ToolButton>
        <span className="pin-tool-separator" />
        <ToolButton label={props.locked ? "Unlock position" : "Lock position"} onClick={props.onToggleLock}>
          {props.locked ? <Lock size={16} /> : <LockOpen size={16} />}
        </ToolButton>
        <ToolButton label="Opacity" onClick={props.onToggleOpacity}>
          <SlidersHorizontal size={16} />
        </ToolButton>
        {props.canEdit && (
          <ToolButton label="Edit image" onClick={props.onEdit}>
            <Edit3 size={16} />
          </ToolButton>
        )}
        {props.canSave && (
          <ToolButton label="Save image" onClick={props.onSave}>
            <Save size={16} />
          </ToolButton>
        )}
        <ToolButton label="Copy" onClick={props.onCopy}>
          {props.copied ? <Check size={16} /> : <Copy size={16} />}
        </ToolButton>
        <ToolButton label="Close" onClick={props.onClose}>
          <X size={16} />
        </ToolButton>
      </div>
      {props.opacityOpen && (
        <div className="pin-opacity-popover">
          <input
            aria-label="Opacity"
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
