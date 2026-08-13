import { renderExport } from "../../react/capture/canvasRenderer";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../../react/capture/imageAdjustments";
import type { Annotation } from "../../react/capture/types";

function context(canvas: HTMLCanvasElement): CanvasRenderingContext2D {
  const value = canvas.getContext("2d", { willReadFrequently: true });
  if (!value) throw new Error("Canvas 2D context is unavailable");
  return value;
}

function pixel(ctx: CanvasRenderingContext2D, x: number, y: number): number[] {
  return Array.from(ctx.getImageData(x, y, 1, 1).data);
}

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function createSource(): HTMLCanvasElement {
  const source = document.createElement("canvas");
  source.width = 8;
  source.height = 8;
  const ctx = context(source);
  ctx.fillStyle = "rgb(200, 30, 20)";
  ctx.fillRect(0, 0, 4, 8);
  ctx.fillStyle = "rgb(40, 80, 120)";
  ctx.fillRect(4, 0, 4, 8);
  return source;
}

function verifyCropAdjustmentsAndRoundedMask(source: HTMLCanvasElement): void {
  const output = document.createElement("canvas");
  output.width = 4;
  output.height = 8;
  const ctx = context(output);
  renderExport(
    ctx,
    source as unknown as HTMLImageElement,
    { x: 4, y: 0, width: 4, height: 8 },
    [],
    {
      ...DEFAULT_IMAGE_ADJUSTMENTS,
      grayscale: true,
      brightness: 20,
      cornerRadius: 2,
    },
  );

  const center = pixel(ctx, 2, 4);
  assert(center[3] === 255, `center alpha: ${center}`);
  assert(Math.max(center[0], center[1], center[2]) - Math.min(center[0], center[1], center[2]) <= 2,
    `grayscale channels: ${center}`);
  assert(center[0] > 80, `brightness was not applied: ${center}`);
  assert(pixel(ctx, 0, 0)[3] < 128, `rounded corner is too opaque: ${pixel(ctx, 0, 0)}`);
}

function verifyVectorAnnotation(source: HTMLCanvasElement): void {
  const output = document.createElement("canvas");
  output.width = 4;
  output.height = 8;
  const ctx = context(output);
  const annotations: Annotation[] = [{
    id: "pixel-rect",
    type: "rect",
    color: "#00ff00",
    size: 1,
    rect: { x: 5, y: 2, width: 2, height: 3 },
  }];
  renderExport(
    ctx,
    source as unknown as HTMLImageElement,
    { x: 4, y: 0, width: 4, height: 8 },
    annotations,
    DEFAULT_IMAGE_ADJUSTMENTS,
  );

  const annotated = pixel(ctx, 1, 2);
  const untouched = pixel(ctx, 3, 6);
  assert(annotated[1] > annotated[0] * 2 && annotated[1] > annotated[2] * 2,
    `annotation pixel was not composited: ${annotated}`);
  assert(untouched[0] === 40 && untouched[1] === 80 && untouched[2] === 120,
    `crop sampled the wrong source pixels: ${untouched}`);
}

try {
  const source = createSource();
  verifyCropAdjustmentsAndRoundedMask(source);
  verifyVectorAnnotation(source);
  document.documentElement.dataset.canvasExport = "passed";
  document.body.style.background = "#00d000";
} catch (error) {
  document.documentElement.dataset.canvasExport = "failed";
  document.body.title = String(error);
  console.error(String(error));
}
