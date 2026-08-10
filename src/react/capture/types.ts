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

export type Tool = "crop" | "object" | "pen" | "rect" | "arrow" | "text" | "blur" | "mosaic";

export type StrokeAnnotation = {
  id: string;
  type: "pen";
  color: string;
  size: number;
  points: Point[];
};

export type RectAnnotation = {
  id: string;
  type: "rect";
  color: string;
  size: number;
  rect: Rect;
};

export type ArrowAnnotation = {
  id: string;
  type: "arrow";
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
};

export type BlurAnnotation = {
  id: string;
  type: "blur";
  rect: Rect;
};

export type MosaicAnnotation = {
  id: string;
  type: "mosaic";
  rect: Rect;
};

export type EffectAnnotation = BlurAnnotation | MosaicAnnotation;

export type Annotation =
  | StrokeAnnotation
  | RectAnnotation
  | ArrowAnnotation
  | TextAnnotation
  | EffectAnnotation;

export type EditorDocument = {
  annotations: Annotation[];
  adjustments: import("./imageAdjustments").ImageAdjustments;
};

export type { CapturedScreenshot } from "../../js/ipc-types.ts";
