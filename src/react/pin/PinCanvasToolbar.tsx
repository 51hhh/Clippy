import { GripHorizontal, Redo2, Trash2, Undo2, X } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { Tool } from "../annotation/types";
import {
  COLORS,
  MAX_STROKE,
  MIN_STROKE,
  TOOL_GROUPS,
} from "../capture-overlay/tools";
import { t } from "../shared/i18n";
import {
  horizontalToolbarPlacement,
  type Box,
  type ToolbarBounds,
} from "../shared/toolbarPlacement";
import { useToolbarDrag } from "../shared/useToolbarDrag";

/** jsdom 与首帧量不到尺寸时的兜底，数量级取自实际布局。 */
const FALLBACK_SIZE = { width: 420, height: 76 };

/**
 * 贴图上画东西时才出现的画布工具条。
 *
 * 工具、颜色、粗细的取值直接用截图覆盖层那份（`capture-overlay/tools`）——两处画的是
 * 同一套标注，分成两份配置的话加一个工具就要改两个地方，而且用户会看到两边工具不一样。
 * 那个模块目前挂在 `capture-overlay/` 下，按位置像是覆盖层私有的，其实是共享的标注配置；
 * 没顺手搬到 `annotation/` 是因为那会动到三个文件加一个测试，与这次的改动无关。
 *
 * `select` 与 `crop` 不在这里：贴图没有"框选"这件事（整张图就是画布），
 * 裁剪也不属于贴图（要裁就保存下来再截）。
 */
const PIN_TOOL_GROUPS = TOOL_GROUPS.map((group) => ({
  ...group,
  tools: group.tools.filter((option) => option.id !== "select"),
})).filter((group) => group.tools.length > 0);

type Props = {
  /** 贴图内容区在窗口里的矩形。工具条优先贴在它下面/上面，都放不下才压上去。 */
  media: Box;
  /** 工具条能待的范围（窗口局部坐标）。**不是窗口尺寸**，见 `ToolbarBounds`。 */
  bounds: ToolbarBounds;
  tool: Tool;
  color: string;
  stroke: number;
  text: string;
  canUndo: boolean;
  canRedo: boolean;
  hasSelectedObject: boolean;
  onTool: (tool: Tool) => void;
  onColor: (color: string) => void;
  onStroke: (stroke: number) => void;
  onText: (text: string) => void;
  onUndo: () => void;
  onRedo: () => void;
  onDeleteObject: () => void;
  onClose: () => void;
};

export function PinCanvasToolbar(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);
  const { position, startDrag } = useToolbarDrag(size, props.bounds);

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

  const auto = horizontalToolbarPlacement(props.media, size, props.bounds);
  const spot = position ?? auto;
  const inside = position ? false : auto.placement === "inside";

  return (
    <div
      ref={panel}
      className={`pin-canvas-toolbar${inside ? " inside" : ""}`}
      data-pin-controls
      style={{ left: spot.left, top: spot.top }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="pin-canvas-row">
        <button
          type="button"
          className="pin-canvas-grip"
          aria-label={t("pin.moveToolbar")}
          title={t("pin.moveToolbar")}
          onPointerDown={(event) => startDrag(event, spot)}
        >
          <GripHorizontal size={14} />
        </button>
        {PIN_TOOL_GROUPS.map((group, index) => (
          <div key={group.titleKey} className="pin-canvas-group">
            {index > 0 && <span className="pin-canvas-separator" />}
            {group.tools.map((option) => (
              <button
                key={option.id}
                type="button"
                className={props.tool === option.id ? "active" : undefined}
                title={t(option.labelKey)}
                aria-label={t(option.labelKey)}
                aria-pressed={props.tool === option.id}
                onClick={() => props.onTool(option.id as Tool)}
              >
                {option.icon}
              </button>
            ))}
          </div>
        ))}
      </div>

      <div className="pin-canvas-row">
        <div className="pin-canvas-colors">
          {COLORS.map((color) => (
            <button
              key={color}
              type="button"
              className={props.color === color ? "pin-canvas-swatch active" : "pin-canvas-swatch"}
              style={{ backgroundColor: color }}
              title={t("capture.color", { color })}
              aria-label={t("capture.color", { color })}
              aria-pressed={props.color === color}
              onClick={() => props.onColor(color)}
            />
          ))}
        </div>
        <label className="pin-canvas-stroke">
          <span>{t("capture.size")}</span>
          <input
            type="range"
            min={MIN_STROKE}
            max={MAX_STROKE}
            value={props.stroke}
            aria-label={t("capture.size")}
            onChange={(event) => props.onStroke(Number(event.target.value))}
          />
        </label>
        <span className="pin-canvas-separator" />
        <button
          type="button"
          title={t("capture.undo")}
          aria-label={t("capture.undo")}
          disabled={!props.canUndo}
          onClick={props.onUndo}
        >
          <Undo2 size={15} />
        </button>
        <button
          type="button"
          title={t("capture.redo")}
          aria-label={t("capture.redo")}
          disabled={!props.canRedo}
          onClick={props.onRedo}
        >
          <Redo2 size={15} />
        </button>
        <button
          type="button"
          title={t("capture.deleteObject")}
          aria-label={t("capture.deleteObject")}
          disabled={!props.hasSelectedObject}
          onClick={props.onDeleteObject}
        >
          <Trash2 size={15} />
        </button>
        <span className="pin-canvas-separator" />
        <button
          type="button"
          title={t("pin.canvasClose")}
          aria-label={t("pin.canvasClose")}
          onClick={props.onClose}
        >
          <X size={15} />
        </button>
      </div>

      {props.tool === "text" && (
        <div className="pin-canvas-row">
          <label className="pin-canvas-text">
            <span>{t("capture.text")}</span>
            <input
              value={props.text}
              aria-label={t("capture.text")}
              onChange={(event) => props.onText(event.target.value)}
            />
          </label>
        </div>
      )}
    </div>
  );
}
