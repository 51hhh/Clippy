import {
  Check,
  Languages,
  Pin,
  Redo2,
  Save,
  SlidersHorizontal,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { ImageAdjustments } from "../annotation/imageAdjustments";
import { t } from "../shared/i18n";
import { toolbarPlacement } from "./geometry";
import { COLORS, MAX_STROKE, MIN_STROKE, TOOL_GROUPS } from "./tools";
import type { CaptureAction, OverlayTool, Rect } from "./types";

/** jsdom 与首帧量不到尺寸时的兜底，数量级取自实际布局。 */
const FALLBACK_SIZE = { width: 552, height: 84 };

type Props = {
  selection: Rect;
  viewportWidth: number;
  viewportHeight: number;
  tool: OverlayTool;
  color: string;
  stroke: number;
  text: string;
  adjustments: ImageAdjustments;
  busy: boolean;
  translationBusy: boolean;
  canUndo: boolean;
  canRedo: boolean;
  hasSelectedObject: boolean;
  onTool: (tool: OverlayTool) => void;
  onColor: (color: string) => void;
  onStroke: (stroke: number) => void;
  onText: (text: string) => void;
  onAdjust: (update: Partial<ImageAdjustments>) => void;
  onUndo: () => void;
  onRedo: () => void;
  onDeleteObject: () => void;
  onAction: (action: CaptureAction) => void;
  onTranslate: () => void;
  onCancel: () => void;
  translateButtonRef: RefObject<HTMLButtonElement>;
};

/**
 * 选区旁边的完整工具条：上排是 16 个工具，下排是样式、撤销/重做与提交动作。
 * 对钩（Check）直接把裁剪 + 标注后的图片复制进剪贴板，不再跳去独立的编辑器窗口。
 */
export function OverlayToolbar(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);
  const [adjustOpen, setAdjustOpen] = useState(false);

  // 工具条的宽高随语言和当前工具变化，测量比写死常量可靠；
  // 没有依赖数组是有意的，靠等值判断避免自激。
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

  const placement = toolbarPlacement(props.selection, size, {
    width: props.viewportWidth,
    height: props.viewportHeight,
  });
  const actionsDisabled = props.busy || props.translationBusy;

  return (
    <div
      ref={panel}
      className="overlay-toolbar"
      style={{ left: placement.left, top: placement.top }}
      onPointerDown={(event) => event.stopPropagation()}
      onPointerMove={(event) => event.stopPropagation()}
    >
      <div className="overlay-toolbar-row">
        {TOOL_GROUPS.map((group, index) => (
          <div key={group.titleKey} className="overlay-tool-group">
            {index > 0 && <span className="overlay-separator" />}
            {group.tools.map((option) => (
              <button
                key={option.id}
                type="button"
                className={props.tool === option.id ? "active" : undefined}
                title={t(option.labelKey)}
                aria-label={t(option.labelKey)}
                aria-pressed={props.tool === option.id}
                onClick={() => props.onTool(option.id)}
              >
                {option.icon}
              </button>
            ))}
          </div>
        ))}
      </div>

      <div className="overlay-toolbar-row">
        <div className="overlay-colors">
          {COLORS.map((color) => (
            <button
              key={color}
              type="button"
              className={props.color === color ? "overlay-swatch active" : "overlay-swatch"}
              style={{ backgroundColor: color }}
              title={t("capture.color", { color })}
              aria-label={t("capture.color", { color })}
              aria-pressed={props.color === color}
              onClick={() => props.onColor(color)}
            />
          ))}
        </div>
        <label className="overlay-stroke">
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
        <span className="overlay-separator" />
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
        <button
          type="button"
          className={adjustOpen ? "active" : undefined}
          title={t("capture.image")}
          aria-label={t("capture.image")}
          aria-pressed={adjustOpen}
          onClick={() => setAdjustOpen((open) => !open)}
        >
          <SlidersHorizontal size={15} />
        </button>
        <span className="overlay-separator" />
        <button
          ref={props.translateButtonRef}
          type="button"
          title={t("capture.translate")}
          aria-label={t("capture.translateSelection")}
          disabled={actionsDisabled}
          onClick={props.onTranslate}
        >
          <Languages size={15} />
        </button>
        <button
          type="button"
          title={t("capture.save")}
          aria-label={t("capture.save")}
          disabled={actionsDisabled}
          onClick={() => props.onAction("save")}
        >
          <Save size={15} />
        </button>
        <button
          type="button"
          title={t("capture.pin")}
          aria-label={t("capture.pin")}
          disabled={actionsDisabled}
          onClick={() => props.onAction("pin")}
        >
          <Pin size={15} />
        </button>
        <button
          type="button"
          className="overlay-confirm"
          title={t("capture.copy")}
          aria-label={t("capture.copy")}
          disabled={actionsDisabled}
          onClick={() => props.onAction("copy")}
        >
          <Check size={16} />
        </button>
        <button
          type="button"
          title={t("capture.cancel")}
          aria-label={t("capture.cancel")}
          disabled={props.busy}
          onClick={props.onCancel}
        >
          <X size={15} />
        </button>
      </div>

      {props.tool === "text" && (
        <div className="overlay-toolbar-row">
          <label className="overlay-text-input">
            <span>{t("capture.text")}</span>
            <input
              value={props.text}
              aria-label={t("capture.text")}
              onChange={(event) => props.onText(event.target.value)}
            />
          </label>
        </div>
      )}

      {adjustOpen && (
        <div className="overlay-toolbar-row overlay-adjustments">
          <label className="overlay-toggle">
            <input
              type="checkbox"
              checked={props.adjustments.grayscale}
              onChange={(event) => props.onAdjust({ grayscale: event.target.checked })}
            />
            <span>{t("capture.grayscale")}</span>
          </label>
          {(["brightness", "contrast", "saturation"] as const).map((key) => (
            <label key={key} className="overlay-stroke">
              <span>{t(`capture.${key}`)}</span>
              <input
                type="range"
                min={-100}
                max={100}
                value={props.adjustments[key]}
                aria-label={t(`capture.${key}`)}
                onChange={(event) => props.onAdjust({ [key]: Number(event.target.value) })}
              />
            </label>
          ))}
          <label className="overlay-stroke">
            <span>{t("capture.corners")}</span>
            <input
              type="range"
              min={0}
              max={120}
              value={props.adjustments.cornerRadius}
              aria-label={t("capture.corners")}
              onChange={(event) => props.onAdjust({ cornerRadius: Number(event.target.value) })}
            />
          </label>
        </div>
      )}
    </div>
  );
}
