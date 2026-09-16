import {
  Check,
  Copy,
  GripHorizontal,
  Lock,
  LockOpen,
  Minus,
  Pencil,
  Pin,
  PinOff,
  Save,
  SlidersHorizontal,
  X,
  Plus,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { t } from "../shared/i18n";
import {
  verticalToolbarPlacement,
  type Box,
  type ToolbarBounds,
} from "../shared/toolbarPlacement";
import { useToolbarDrag } from "../shared/useToolbarDrag";

/** jsdom 与首帧量不到尺寸时的兜底，数量级取自实际布局（38 宽 + 九个 28 高的按钮）。 */
const FALLBACK_SIZE = { width: 38, height: 249 };

type Props = {
  /** 贴图内容区在窗口里的矩形。工具条优先贴在它外面，放不下才压上去。 */
  media: Box;
  /** 工具条能待的范围（窗口局部坐标）。**不是窗口尺寸**，见 `ToolbarBounds`。 */
  bounds: ToolbarBounds;
  scale: number;
  opacity: number;
  locked: boolean;
  above: boolean;
  aboveSupported: boolean;
  aboveLimited: boolean;
  canvasOpen: boolean;
  canSave: boolean;
  copied: boolean;
  opacityOpen: boolean;
  onScale: (scale: number) => void;
  onOpacity: (opacity: number) => void;
  onToggleOpacity: () => void;
  onToggleLock: () => void;
  onToggleAbove: () => void;
  onToggleCanvas: () => void;
  onCopy: () => void;
  onSave: () => void;
  onClose: () => void;
};

function ToolButton({
  label,
  onClick,
  active,
  children,
}: {
  label: string;
  onClick: () => void;
  /** 开关类按钮的按下态。`aria-pressed` 让读屏软件也能读出开关状态。 */
  active?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className={`pin-tool-button${active ? " active" : ""}`}
      aria-label={label}
      aria-pressed={active}
      title={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

export function PinToolbar(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);
  const { position, startDrag } = useToolbarDrag(size, props.bounds);

  // 工具条的高度随按钮增减变化（保存按钮只在可保存时出现，画布开着时多一行），
  // 量出来比写死常量可靠。没有依赖数组是有意的，靠等值判断避免自激——
  // 与 `OverlayToolbar` 同一套写法。
  useLayoutEffect(() => {
    const element = panel.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const next = {
      width: rect.width || FALLBACK_SIZE.width,
      height: rect.height || FALLBACK_SIZE.height,
    };
    setSize((current) =>
      current.width === next.width && current.height === next.height ? current : next,
    );
  });

  // 用户拖过就听用户的；没拖过则自动选边（右 → 左 → 内部）。
  const auto = verticalToolbarPlacement(props.media, size, props.bounds);
  const spot = position ?? auto;
  const inside = position ? false : auto.placement === "inside";

  return (
    <div
      ref={panel}
      className={`pin-controls${inside ? " inside" : ""}`}
      data-pin-controls
      style={{ left: spot.left, top: spot.top }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="pin-tools-vertical">
        <button
          type="button"
          className="pin-tool-grip"
          aria-label={t("pin.moveToolbar")}
          title={t("pin.moveToolbar")}
          onPointerDown={(event) => startDrag(event, spot)}
        >
          <GripHorizontal size={14} />
        </button>
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
        {props.aboveSupported && (
          <ToolButton
            label={t(
              props.above
                ? props.aboveLimited
                  ? "pin.unpinAboveLimited"
                  : "pin.unpinAbove"
                : props.aboveLimited
                  ? "pin.pinAboveLimited"
                  : "pin.pinAbove",
            )}
            active={props.above}
            onClick={props.onToggleAbove}
          >
            {props.above ? <Pin size={16} /> : <PinOff size={16} />}
          </ToolButton>
        )}
        <ToolButton
          label={t(props.locked ? "pin.unlock" : "pin.lock")}
          active={props.locked}
          onClick={props.onToggleLock}
        >
          {props.locked ? <Lock size={16} /> : <LockOpen size={16} />}
        </ToolButton>
        {props.canSave && (
          <ToolButton
            label={t(props.canvasOpen ? "pin.canvasClose" : "pin.canvasOpen")}
            active={props.canvasOpen}
            onClick={props.onToggleCanvas}
          >
            <Pencil size={16} />
          </ToolButton>
        )}
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
