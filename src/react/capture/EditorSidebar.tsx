import {
  ArrowUpRight,
  Blend,
  Crop,
  Grid3X3,
  MousePointer2,
  PenLine,
  RectangleHorizontal,
  Type,
  Trash2,
} from "lucide-react";
import type { ReactNode } from "react";
import type { ImageAdjustments } from "./imageAdjustments";
import type { Tool } from "./types";

const TOOLS: Array<{ id: Tool; label: string; icon: ReactNode }> = [
  { id: "crop", label: "Crop", icon: <Crop size={16} /> },
  { id: "object", label: "Object", icon: <MousePointer2 size={16} /> },
  { id: "pen", label: "Pen", icon: <PenLine size={16} /> },
  { id: "rect", label: "Rectangle", icon: <RectangleHorizontal size={16} /> },
  { id: "arrow", label: "Arrow", icon: <ArrowUpRight size={16} /> },
  { id: "text", label: "Text", icon: <Type size={16} /> },
  { id: "blur", label: "Blur", icon: <Blend size={16} /> },
  { id: "mosaic", label: "Mosaic", icon: <Grid3X3 size={16} /> },
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
        <div className="tool-group-title">Tools</div>
        <div className="segmented-tools">
          {TOOLS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={props.tool === option.id ? "tool-button active" : "tool-button"}
              onClick={() => props.onTool(option.id)}
              title={option.label}
            >
              {option.icon}
              <span>{option.label}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="tool-group">
        <div className="tool-group-title">Style</div>
        <div className="color-row">
          {COLORS.map((color) => (
            <button
              key={color}
              type="button"
              className={props.color === color ? "color-swatch active" : "color-swatch"}
              style={{ backgroundColor: color }}
              onClick={() => props.onColor(color)}
              title={color}
              aria-label={`Color ${color}`}
            />
          ))}
        </div>
        <RangeControl label="Size" min={2} max={16} value={props.size} onChange={props.onSize} />
        <label className="text-control">
          <span>Text</span>
          <input value={props.text} onChange={(event) => props.onText(event.target.value)} />
        </label>
        <button
          type="button"
          className="capture-btn delete-annotation"
          disabled={!props.hasSelection}
          onClick={props.onDelete}
        >
          <Trash2 size={15} /> Delete object
        </button>
      </div>

      <div className="tool-group">
        <div className="tool-group-title">Image</div>
        <label className="toggle-row">
          <input
            type="checkbox"
            checked={props.adjustments.grayscale}
            onChange={(event) => props.onAdjust({ grayscale: event.target.checked })}
          />
          <span>Grayscale</span>
        </label>
        {(["brightness", "contrast", "saturation"] as const).map((key) => (
          <RangeControl
            key={key}
            label={key[0].toUpperCase() + key.slice(1)}
            min={-100}
            max={100}
            value={props.adjustments[key]}
            onChange={(value) => props.onAdjust({ [key]: value })}
          />
        ))}
        <RangeControl
          label="Corners"
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
