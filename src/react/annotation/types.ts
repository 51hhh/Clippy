export type Point = {
  x: number;
  y: number;
};

export type Rect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type Tool =
  | "crop"
  | "object"
  | "eraser"
  | "pen"
  | "marker"
  | "rect"
  | "ellipse"
  | "highlight"
  | "line"
  | "arrow"
  | "measure"
  | "text"
  | "blur"
  | "mosaic"
  | "spotlight"
  | "magnifier";

/** 自由手绘：`marker` 是半透明粗笔，其余与 `pen` 完全同构 */
export type StrokeAnnotation = {
  id: string;
  type: "pen" | "marker";
  color: string;
  size: number;
  points: Point[];
};

/** 由拖拽矩形定义的图形；`highlight` 是半透明填充，`ellipse` 画内切椭圆 */
export type ShapeAnnotation = {
  id: string;
  type: "rect" | "ellipse" | "highlight";
  color: string;
  size: number;
  rect: Rect;
};

/** 两点线段；`arrow` 带箭头，`measure` 带端点刻度与像素长度标注 */
export type SegmentAnnotation = {
  id: string;
  type: "arrow" | "line" | "measure";
  color: string;
  size: number;
  from: Point;
  to: Point;
};

export type TextAnnotation = {
  id: string;
  type: "text";
  color: string;
  size: number;
  at: Point;
  text: string;
  /**
   * 渲染器 v1 固定为系统 UI 字体策略，但不固定具体字体文件或字形。
   * 未修改工程复用 IDAT 时像素不变；跨系统继续编辑触发重绘后，历史文字仍可能因
   * 字体、字宽和抗锯齿不同而变化。要保证像素级一致需由后续渲染器保存字形或栅格快照。
   */
  fontFamily?: "system-ui";
};

/**
 * 需要读取底图像素或压暗底图的注解。它们没有颜色和线宽，
 * 且必须在矢量注解之前绘制，否则会盖掉画在同一区域上的标注。
 */
export const EFFECT_TYPES = ["blur", "mosaic", "spotlight", "magnifier"] as const;

export type EffectType = (typeof EFFECT_TYPES)[number];

export type EffectParameters = {
  blurRadius: number;
  mosaicCell: number;
  spotlightDim: number;
  magnifierZoom: number;
};

export const DEFAULT_EFFECT_PARAMETERS: EffectParameters = {
  blurRadius: 8,
  mosaicCell: 12,
  spotlightDim: 0.55,
  magnifierZoom: 2,
};

export type EffectAnnotation = {
  id: string;
  type: EffectType;
  rect: Rect;
  /** rendererVersion=1 的外观参数；旧的内存对象缺席时渲染器使用同一组默认值。 */
  effect?: EffectParameters;
};

export type VectorAnnotation =
  | StrokeAnnotation
  | ShapeAnnotation
  | SegmentAnnotation
  | TextAnnotation;

export type Annotation = VectorAnnotation | EffectAnnotation;

export type EditorDocument = {
  rendererVersion: 1 | 2;
  sourceWidth: number;
  sourceHeight: number;
  annotations: Annotation[];
  adjustments: import("./imageAdjustments").ImageAdjustments;
};
