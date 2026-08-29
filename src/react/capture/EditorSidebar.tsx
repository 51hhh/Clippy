import {
  ArrowUpRight,
  Blend,
  Circle,
  Crop,
  Eraser,
  Flashlight,
  Grid3X3,
  Highlighter,
  Minus,
  MousePointer2,
  PaintBucket,
  PenLine,
  RectangleHorizontal,
  Ruler,
  Type,
  Trash2,
  ZoomIn,
} from "lucide-react";
import type { ReactNode } from "react";
import type { ImageAdjustments } from "./imageAdjustments";
import type { Tool } from "./types";
import { t } from "../shared/i18n";

type ToolOption = { id: Tool; labelKey: string; icon: ReactNode };

/** 工具按分类分组：16 个按钮平铺在侧栏里认不出主次 */
export const TOOL_GROUPS: Array<{ titleKey: string; tools: ToolOption[] }> = [
  {
    titleKey: "capture.toolGroup.select",
    tools: [
      { id: "crop", labelKey: "capture.tool.crop", icon: <Crop size={16} /> },
      { id: "object", labelKey: "capture.tool.object", icon: <MousePointer2 size={16} /> },
      { id: "eraser", labelKey: "capture.tool.eraser", icon: <Eraser size={16} /> },
    ],
  },
  {
    titleKey: "capture.toolGroup.draw",
    tools: [
      { id: "pen", labelKey: "capture.tool.pen", icon: <PenLine size={16} /> },
      { id: "marker", labelKey: "capture.tool.marker", icon: <Highlighter size={16} /> },
      { id: "rect", labelKey: "capture.tool.rectangle", icon: <RectangleHorizontal size={16} /> },
      { id: "ellipse", labelKey: "capture.tool.ellipse", icon: <Circle size={16} /> },
      { id: "line", labelKey: "capture.tool.line", icon: <Minus size={16} /> },
      { id: "arrow", labelKey: "capture.tool.arrow", icon: <ArrowUpRight size={16} /> },
      { id: "measure", labelKey: "capture.tool.measure", icon: <Ruler size={16} /> },
      { id: "text", labelKey: "capture.tool.text", icon: <Type size={16} /> },
    ],
  },
  {
    titleKey: "capture.toolGroup.effects",
    tools: [
      { id: "highlight", labelKey: "capture.tool.highlight", icon: <PaintBucket size={16} /> },
      { id: "blur", labelKey: "capture.tool.blur", icon: <Blend size={16} /> },
      { id: "mosaic", labelKey: "capture.tool.mosaic", icon: <Grid3X3 size={16} /> },
      { id: "spotlight", labelKey: "capture.tool.spotlight", icon: <Flashlight size={16} /> },
      { id: "magnifier", labelKey: "capture.tool.magnifier", icon: <ZoomIn size={16} /> },
    ],
  },
];

const COLORS = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff", "#111111"];

type Props = {
  tool: Tool;
  color: string;
  size: number;
  text: string;
  adjustments: ImageAdjustments;
  hasSelection: boolean;
  onTool: (tool: Tool) => void;
  onColor: (color: string) => void;
  onSize: (size: number) => void;
  onText: (text: string) => void;
  onAdjust: (update: Partial<ImageAdjustments>) => void;
  onDelete: () => void;
};

export function EditorSidebar(props: Props) {
  return (
    <aside className="capture-sidebar">
      <div className="tool-group">
        <div className="tool-group-title">{t("capture.tools")}</div>
        {TOOL_GROUPS.map((group) => (
          <div key={group.titleKey} className="tool-subgroup">
            <div className="tool-subgroup-title">{t(group.titleKey)}</div>
            <div className="segmented-tools">
              {group.tools.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className={props.tool === option.id ? "tool-button active" : "tool-button"}
                  onClick={() => props.onTool(option.id)}
                  title={t(option.labelKey)}
                >
                  {option.icon}
                  <span>{t(option.labelKey)}</span>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>

      <div className="tool-group">
        <div className="tool-group-title">{t("capture.style")}</div>
        <div className="color-row">
          {COLORS.map((color) => (
            <button
              key={color}
              type="button"
              className={props.color === color ? "color-swatch active" : "color-swatch"}
              style={{ backgroundColor: color }}
              onClick={() => props.onColor(color)}
              title={color}
              aria-label={t("capture.color", { color })}
            />
          ))}
        </div>
        <RangeControl label={t("capture.size")} min={2} max={16} value={props.size} onChange={props.onSize} />
        <label className="text-control">
          <span>{t("capture.text")}</span>
          <input value={props.text} onChange={(event) => props.onText(event.target.value)} />
        </label>
        <button
          type="button"
          className="capture-btn delete-annotation"
          disabled={!props.hasSelection}
          onClick={props.onDelete}
        >
          <Trash2 size={15} /> {t("capture.deleteObject")}
        </button>
      </div>

      <div className="tool-group">
        <div className="tool-group-title">{t("capture.image")}</div>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={props.adjustments.grayscale}
            onChange={(event) => props.onAdjust({ grayscale: event.target.checked })}
          />
          <span>{t("capture.grayscale")}</span>
        </label>
        {(["brightness", "contrast", "saturation"] as const).map((key) => (
          <RangeControl
            key={key}
            label={t(`capture.${key}`)}
            min={-100}
            max={100}
            value={props.adjustments[key]}
            onChange={(value) => props.onAdjust({ [key]: value })}
          />
        ))}
        <RangeControl
          label={t("capture.corners")}
          min={0}
          max={120}
          value={props.adjustments.cornerRadius}
          onChange={(cornerRadius) => props.onAdjust({ cornerRadius })}
        />
      </div>
    </aside>
  );
}

function RangeControl({
  label,
  min,
  max,
  value,
  onChange,
}: {
  label: string;
  min: number;
  max: number;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="control-row">
      <span>{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <output>{value}</output>
    </label>
  );
}
