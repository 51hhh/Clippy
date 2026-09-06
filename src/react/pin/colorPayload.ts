import type { PinColor, PinPayload } from "../../js/ipc-types.ts";

type ColorPinPayload = PinPayload & {
  kind: "color";
  text: string;
  color: PinColor;
};

const PAYLOAD_FIELDS = [
  "label", "kind", "text", "color", "contentWidth", "contentHeight", "scale", "opacity",
  "locked", "above", "canSave", "position", "deviceScale", "bufferScale", "initialProject",
] as const;
const COLOR_FIELDS = ["red", "green", "blue", "alpha", "canonical"] as const;

function hasExactFields(value: Record<string, unknown>, fields: readonly string[]): boolean {
  const keys = Object.keys(value);
  return keys.length === fields.length && keys.every((key) => fields.includes(key));
}

function isByte(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 255;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isPosition(value: unknown): boolean {
  if (value === null) return true;
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const position = value as Record<string, unknown>;
  return hasExactFields(position, ["x", "y"]) && isFiniteNumber(position.x) && isFiniteNumber(position.y);
}

/**
 * `get_pin_payload` 是 IPC 信任边界。颜色值绝不能由 WebView 解析：只接收后端已规范化的
 * RGBA/canonical 对，并在写 React state 前逐字段验证，避免恶意文本进入 CSS 或内容区。
 */
export function isColorPinPayload(value: unknown): value is ColorPinPayload {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const payload = value as Record<string, unknown>;
  if (!hasExactFields(payload, PAYLOAD_FIELDS) || payload.kind !== "color") return false;
  if (
    typeof payload.label !== "string"
    || typeof payload.text !== "string"
    || payload.canSave !== false
    || payload.initialProject !== null
    || typeof payload.locked !== "boolean"
    || typeof payload.above !== "boolean"
    || !isFiniteNumber(payload.contentWidth)
    || !isFiniteNumber(payload.contentHeight)
    || !isFiniteNumber(payload.scale)
    || !isFiniteNumber(payload.opacity)
    || !isFiniteNumber(payload.deviceScale)
    || !isFiniteNumber(payload.bufferScale)
    || !isPosition(payload.position)
  ) return false;

  if (!payload.color || typeof payload.color !== "object" || Array.isArray(payload.color)) return false;
  const color = payload.color as Record<string, unknown>;
  if (!hasExactFields(color, COLOR_FIELDS) || !COLOR_FIELDS.slice(0, 4).every((key) => isByte(color[key]))) {
    return false;
  }
  if (typeof color.canonical !== "string" || !/^#[0-9a-f]{8}$/.test(color.canonical)) return false;
  const canonical = `#${[color.red, color.green, color.blue, color.alpha]
    .map((channel) => (channel as number).toString(16).padStart(2, "0"))
    .join("")}`;
  return color.canonical === canonical;
}
