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

export type Tool = "select" | "pen" | "rect" | "arrow" | "text";

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

export type Annotation =
  | StrokeAnnotation
  | RectAnnotation
  | ArrowAnnotation
  | TextAnnotation;

export type CapturedScreenshot = {
  pngBase64: string;
  width: number;
  height: number;
};

