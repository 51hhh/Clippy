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
  ZoomIn,
} from "lucide-react";
import type { ReactNode } from "react";
import type { OverlayTool } from "./types";

type ToolOption = { id: OverlayTool; labelKey: string; icon: ReactNode };

/**
 * 覆盖层工具条的分组。分组沿用旧编辑器侧栏的三档（选择 / 绘制 / 效果），
 * 只把 `crop` 换成 `select`：选区本身就是裁剪框，由选区的手柄直接拖，
 * 不需要再有一个"裁剪工具"。
 */
export const TOOL_GROUPS: Array<{ titleKey: string; tools: ToolOption[] }> = [
  {
    titleKey: "capture.toolGroup.select",
    tools: [
      { id: "select", labelKey: "capture.tool.selection", icon: <Crop size={15} /> },
      { id: "object", labelKey: "capture.tool.object", icon: <MousePointer2 size={15} /> },
      { id: "eraser", labelKey: "capture.tool.eraser", icon: <Eraser size={15} /> },
    ],
  },
  {
    titleKey: "capture.toolGroup.draw",
    tools: [
      { id: "pen", labelKey: "capture.tool.pen", icon: <PenLine size={15} /> },
      { id: "marker", labelKey: "capture.tool.marker", icon: <Highlighter size={15} /> },
      { id: "rect", labelKey: "capture.tool.rectangle", icon: <RectangleHorizontal size={15} /> },
      { id: "ellipse", labelKey: "capture.tool.ellipse", icon: <Circle size={15} /> },
      { id: "line", labelKey: "capture.tool.line", icon: <Minus size={15} /> },
      { id: "arrow", labelKey: "capture.tool.arrow", icon: <ArrowUpRight size={15} /> },
      { id: "measure", labelKey: "capture.tool.measure", icon: <Ruler size={15} /> },
      { id: "text", labelKey: "capture.tool.text", icon: <Type size={15} /> },
    ],
  },
  {
    titleKey: "capture.toolGroup.effects",
    tools: [
      { id: "highlight", labelKey: "capture.tool.highlight", icon: <PaintBucket size={15} /> },
      { id: "blur", labelKey: "capture.tool.blur", icon: <Blend size={15} /> },
      { id: "mosaic", labelKey: "capture.tool.mosaic", icon: <Grid3X3 size={15} /> },
      { id: "spotlight", labelKey: "capture.tool.spotlight", icon: <Flashlight size={15} /> },
      { id: "magnifier", labelKey: "capture.tool.magnifier", icon: <ZoomIn size={15} /> },
    ],
  },
];

/** 标注颜色。第一个是默认色。 */
export const COLORS = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff", "#111111"];

export const DEFAULT_COLOR = COLORS[0];
export const MIN_STROKE = 2;
export const MAX_STROKE = 16;
export const DEFAULT_STROKE = 4;
