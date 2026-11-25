import { DEFAULT_IMAGE_ADJUSTMENTS, type ImageAdjustments } from "../annotation/imageAdjustments";
import {
  DEFAULT_EFFECT_PARAMETERS,
  EFFECT_TYPES,
  type Annotation,
  type EditorDocument,
  type EffectParameters,
  type Point,
  type Rect,
} from "../annotation/types";

const MAX_ANNOTATIONS = 10_000;
const MAX_STROKE_POINTS = 100_000;
const MAX_TOTAL_POINTS = 500_000;
const MAX_TEXT_LENGTH = 16 * 1024;
const MAX_ID_LENGTH = 128;
const VECTOR_TYPES = new Set(["pen", "marker", "rect", "ellipse", "highlight", "arrow", "line", "measure", "text"]);
const EFFECT_TYPE_SET = new Set<string>(EFFECT_TYPES);
const HEX_COLOR = /^#[0-9a-f]{3,4}(?:[0-9a-f]{3,4})?$/i;

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function finite(value: unknown, min: number, max: number): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= min && value <= max;
}

function point(value: unknown, width: number, height: number): Point | null {
  const item = record(value);
  return item && finite(item.x, 0, width) && finite(item.y, 0, height)
    ? { x: item.x, y: item.y }
    : null;
}

function rect(value: unknown, width: number, height: number): Rect | null {
  const item = record(value);
  if (!item || !finite(item.x, 0, width) || !finite(item.y, 0, height)
    || !finite(item.width, 0, width) || !finite(item.height, 0, height)
    || item.x + item.width > width || item.y + item.height > height) return null;
  return { x: item.x, y: item.y, width: item.width, height: item.height };
}

function effectParameters(value: unknown): EffectParameters | null {
  if (value === undefined) return { ...DEFAULT_EFFECT_PARAMETERS };
  const item = record(value);
  if (!item || !finite(item.blurRadius, 1, 100) || !finite(item.mosaicCell, 1, 256)
    || !finite(item.spotlightDim, 0, 1) || !finite(item.magnifierZoom, 1, 16)) return null;
  return {
    blurRadius: item.blurRadius,
    mosaicCell: item.mosaicCell,
    spotlightDim: item.spotlightDim,
    magnifierZoom: item.magnifierZoom,
  };
}

function adjustments(value: unknown): ImageAdjustments | null {
  const item = record(value);
  if (!item || typeof item.grayscale !== "boolean"
    || !finite(item.brightness, -100, 100) || !finite(item.contrast, -100, 100)
    || !finite(item.saturation, -100, 100) || !finite(item.cornerRadius, 0, 120)) return null;
  return {
    grayscale: item.grayscale,
    brightness: item.brightness,
    contrast: item.contrast,
    saturation: item.saturation,
    cornerRadius: item.cornerRadius,
  };
}

function annotation(
  value: unknown,
  width: number,
  height: number,
): { value: Annotation; pointCount: number } | null {
  const item = record(value);
  if (!item || typeof item.id !== "string" || item.id.length === 0 || item.id.length > MAX_ID_LENGTH
    || typeof item.type !== "string") return null;
  const id = item.id;
  const type = item.type;
  if (EFFECT_TYPE_SET.has(type)) {
    const bounds = rect(item.rect, width, height);
    const effect = effectParameters(item.effect);
    return bounds && effect
      ? { value: { id, type: type as Annotation["type"] & ("blur" | "mosaic" | "spotlight" | "magnifier"), rect: bounds, effect }, pointCount: 0 }
      : null;
  }
  if (!VECTOR_TYPES.has(type) || typeof item.color !== "string" || !HEX_COLOR.test(item.color)
    || !finite(item.size, 0.1, 128)) return null;
  const common = { id, color: item.color, size: item.size };
  if (type === "pen" || type === "marker") {
    if (!Array.isArray(item.points) || item.points.length > MAX_STROKE_POINTS) return null;
    const points = item.points.map((candidate) => point(candidate, width, height));
    return points.every((candidate): candidate is Point => candidate !== null)
      ? { value: { ...common, type, points }, pointCount: points.length }
      : null;
  }
  if (type === "rect" || type === "ellipse" || type === "highlight") {
    const bounds = rect(item.rect, width, height);
    return bounds ? { value: { ...common, type, rect: bounds }, pointCount: 0 } : null;
  }
  if (type === "arrow" || type === "line" || type === "measure") {
    const from = point(item.from, width, height);
    const to = point(item.to, width, height);
    return from && to ? { value: { ...common, type, from, to }, pointCount: 2 } : null;
  }
  const at = point(item.at, width, height);
  return at && typeof item.text === "string" && item.text.length <= MAX_TEXT_LENGTH
    && (item.fontFamily === undefined || item.fontFamily === "system-ui")
    ? { value: { ...common, type: "text", at, text: item.text, fontFamily: "system-ui" }, pointCount: 1 }
    : null;
}

/**
 * IPC 仍是一个信任边界：即使 Rust 已校验，也只把逐字段确认过的数据放进 React state。
 * 无效工程返回 null，调用方保留 PNG 的 IDAT 扁平预览。
 */
export function parseInitialPinProject(value: unknown): EditorDocument | null {
  const project = record(value);
  const source = record(project?.source);
  const document = record(project?.document);
  if (!project || project.format !== "clippy-pin-project"
    || (project.formatVersion !== 2 && project.formatVersion !== 3)
    || (project.rendererVersion !== 1 && project.rendererVersion !== 2) || !source || !document
    || !Number.isInteger(source.width) || !finite(source.width, 1, 100_000)
    || !Number.isInteger(source.height) || !finite(source.height, 1, 100_000)
    || typeof source.sha256 !== "string" || !/^[a-f0-9]{64}$/i.test(source.sha256)
    || !Array.isArray(document.annotations) || document.annotations.length > MAX_ANNOTATIONS) return null;
  const imageAdjustments = adjustments(document.adjustments);
  if (!imageAdjustments) return null;
  const ids = new Set<string>();
  const annotations: Annotation[] = [];
  let totalPoints = 0;
  for (const candidate of document.annotations) {
    const parsed = annotation(candidate, source.width, source.height);
    if (!parsed || ids.has(parsed.value.id)) return null;
    ids.add(parsed.value.id);
    totalPoints += parsed.pointCount;
    if (totalPoints > MAX_TOTAL_POINTS) return null;
    annotations.push(parsed.value);
  }
  return {
    rendererVersion: project.rendererVersion,
    sourceWidth: source.width,
    sourceHeight: source.height,
    annotations,
    adjustments: imageAdjustments,
  };
}

export function emptyEditorDocument(width: number, height: number): EditorDocument {
  return {
    rendererVersion: 2,
    sourceWidth: width,
    sourceHeight: height,
    annotations: [],
    adjustments: { ...DEFAULT_IMAGE_ADJUSTMENTS },
  };
}
