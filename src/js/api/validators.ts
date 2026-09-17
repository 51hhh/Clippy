import type { StructuredOcr } from "../ipc-types.ts";

export type ImageCodePoint = {
  x: number;
  y: number;
};

export type DetectedImageCode = {
  format: "qr_code" | "code_39";
  text: string;
  points: ImageCodePoint[];
};

export type ImageCodeScanResponse = {
  results: DetectedImageCode[];
  limited: boolean;
};

const MAX_IMAGE_CODE_RESULTS = 32;
const MAX_IMAGE_CODE_POINTS = 64;
const MAX_IMAGE_CODE_TEXT_BYTES = 16 * 1024;
const MAX_IMAGE_CODE_TOTAL_TEXT_BYTES = 64 * 1024;
const MAX_IMAGE_CODE_COORDINATE = 16_384;
const imageCodeTextEncoder = new TextEncoder();

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function hasExactKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every((key) => Object.hasOwn(value, key));
}

/**
 * 从 IPC 返回的未知 JSON 中恢复受限的本地扫码结果。
 *
 * 后端已经按相同预算限制输出；前端仍在边界逐字段校验，避免被损坏或陈旧的 IPC
 * payload 直接写入 DOM。文本原样保留（包括 NUL 和双向控制字符），调用方必须使用
 * textContent 渲染。
 */
export function parseImageCodeScanResponse(value: unknown): ImageCodeScanResponse {
  if (!isPlainRecord(value) || !hasExactKeys(value, ["results", "limited"])) {
    throw new Error("invalid image code scan response");
  }
  if (!Array.isArray(value.results) || value.results.length > MAX_IMAGE_CODE_RESULTS) {
    throw new Error("invalid image code scan results");
  }
  if (typeof value.limited !== "boolean") {
    throw new Error("invalid image code scan limit");
  }

  let totalTextBytes = 0;
  const results = value.results.map((candidate) => {
    if (!isPlainRecord(candidate) || !hasExactKeys(candidate, ["format", "text", "points"])) {
      throw new Error("invalid image code scan result");
    }
    if (candidate.format !== "qr_code" && candidate.format !== "code_39") {
      throw new Error("invalid image code scan format");
    }
    const format: DetectedImageCode["format"] = candidate.format;
    if (typeof candidate.text !== "string") {
      throw new Error("invalid image code scan text");
    }
    const textBytes = imageCodeTextEncoder.encode(candidate.text).byteLength;
    if (textBytes > MAX_IMAGE_CODE_TEXT_BYTES) {
      throw new Error("invalid image code scan text length");
    }
    totalTextBytes += textBytes;
    if (totalTextBytes > MAX_IMAGE_CODE_TOTAL_TEXT_BYTES) {
      throw new Error("invalid image code scan total text length");
    }
    if (!Array.isArray(candidate.points) || candidate.points.length > MAX_IMAGE_CODE_POINTS) {
      throw new Error("invalid image code scan points");
    }
    const points = candidate.points.map((point) => {
      if (!isPlainRecord(point) || !hasExactKeys(point, ["x", "y"])) {
        throw new Error("invalid image code scan point");
      }
      if (
        typeof point.x !== "number"
        || typeof point.y !== "number"
        || !Number.isFinite(point.x)
        || !Number.isFinite(point.y)
        || point.x < 0
        || point.y < 0
        || point.x > MAX_IMAGE_CODE_COORDINATE
        || point.y > MAX_IMAGE_CODE_COORDINATE
      ) {
        throw new Error("invalid image code scan point coordinate");
      }
      return { x: point.x, y: point.y };
    });
    return { format, text: candidate.text, points };
  });

  return { results, limited: value.limited };
}

export function checkedStructuredOcr(value: StructuredOcr): StructuredOcr {
  const probability = (number: number) => Number.isFinite(number) && number >= 0 && number <= 1;
  if (!value || !Number.isInteger(value.width) || !Number.isInteger(value.height) || value.width < 1 || value.height < 1
    || value.width > 16384 || value.height > 16384 || typeof value.text !== "string" || value.text.length > 1_048_576
    || !Array.isArray(value.lines) || value.lines.length > 512 || !Array.isArray(value.paragraphs) || value.paragraphs.length > value.lines.length
    || !["ppocrv6+edgegnn", "tesseract"].includes(value.pipeline?.engine) || typeof value.pipeline.id !== "string"
    || typeof value.pipeline.layoutExecuted !== "boolean" || !(value.fallbackReason === null || typeof value.fallbackReason === "string")) throw new Error("viewer.invalid_ocr");
  let total = 0;
  for (const line of value.lines) {
    if (!line || typeof line.text !== "string" || line.text.length > 65536 || typeof line.accepted !== "boolean"
      || !Number.isInteger(line.id) || !Number.isInteger(line.readingOrder) || !Number.isInteger(line.paragraphId)
      || !probability(line.confidence) || !Array.isArray(line.charConfidences) || line.charConfidences.length > 65536
      || line.charConfidences.some(number => !probability(number)) || !Array.isArray(line.quad) || line.quad.length !== 4
      || line.quad.some(point => !Array.isArray(point) || point.length !== 2 || !Number.isFinite(point[0]) || !Number.isFinite(point[1]) || point[0] < 0 || point[0] > value.width || point[1] < 0 || point[1] > value.height)) throw new Error("viewer.invalid_ocr");
    total += line.text.length; if (total > 1_048_576) throw new Error("viewer.invalid_ocr");
  }
  for (const paragraph of value.paragraphs) {
    if (!paragraph || !Number.isInteger(paragraph.id) || !Number.isInteger(paragraph.readingOrder) || !Array.isArray(paragraph.lineIds)
      || paragraph.lineIds.length > 512 || paragraph.lineIds.some(id => !Number.isInteger(id))) throw new Error("viewer.invalid_ocr");
  }
  return value;
}
