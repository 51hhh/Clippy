import { renderExport } from "./canvasRenderer";
import type { FrameImage } from "./frameImage";
import type { ImageAdjustments } from "./imageAdjustments";
import type { Annotation, Rect } from "./types";

const PNG_DATA_URL_PREFIX = /^data:image\/png;base64,/;

export function isExportSelection(selection: Rect | null): selection is Rect {
  return !!selection && selection.width >= 3 && selection.height >= 3;
}

export function stripPngDataUrl(value: string): string {
  return value.replace(PNG_DATA_URL_PREFIX, "");
}

export function pngBase64ToBytes(pngBase64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(pngBase64);
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function pngBase64ToObjectUrl(pngBase64: string): string {
  return URL.createObjectURL(new Blob([pngBase64ToBytes(pngBase64)], { type: "image/png" }));
}

export function blobToPngBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => resolve(stripPngDataUrl(String(reader.result || "")));
    reader.readAsDataURL(blob);
  });
}

function canvasToPngBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((result) => {
      if (result) resolve(result);
      else reject(new Error("Failed to export PNG"));
    }, "image/png");
  });
}

export async function exportPngBase64(
  image: FrameImage,
  crop: Rect,
  annotations: Annotation[],
  adjustments: ImageAdjustments,
): Promise<string> {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(crop.width));
  canvas.height = Math.max(1, Math.round(crop.height));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is not available");

  renderExport(ctx, image, crop, annotations, adjustments);
  return blobToPngBase64(await canvasToPngBlob(canvas));
}
