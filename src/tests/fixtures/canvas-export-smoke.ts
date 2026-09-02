import { drawScene, renderExport } from "../../react/annotation/canvasRenderer";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../../react/annotation/imageAdjustments";
import type { Annotation } from "../../react/annotation/types";

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

function rgbDistance(left: number[], right: number[]): number {
  return Math.abs(left[0] - right[0])
    + Math.abs(left[1] - right[1])
    + Math.abs(left[2] - right[2]);
}

function sourcePixel(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  x: number,
  y: number,
  sourceWidth: number,
  sourceHeight: number,
): number[] {
  return pixel(
    ctx,
    Math.min(canvas.width - 1, Math.max(0, Math.floor(x * canvas.width / sourceWidth))),
    Math.min(canvas.height - 1, Math.max(0, Math.floor(y * canvas.height / sourceHeight))),
  );
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
  // 效果类注解读的是 naturalWidth/naturalHeight，canvas 当替身时要补上。
  return Object.assign(source, { naturalWidth: 8, naturalHeight: 8 });
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

/** 高亮矩形必须是半透明合成，聚光灯必须只压暗选区之外 */
function verifyTranslucentAndDimmingEffects(source: HTMLCanvasElement): void {
  const output = document.createElement("canvas");
  output.width = 8;
  output.height = 8;
  const ctx = context(output);
  const annotations: Annotation[] = [
    { id: "spot", type: "spotlight", rect: { x: 0, y: 0, width: 4, height: 8 } },
    { id: "mark", type: "highlight", color: "#00ff00", size: 2, rect: { x: 4, y: 0, width: 4, height: 8 } },
  ];
  renderExport(
    ctx,
    source as unknown as HTMLImageElement,
    { x: 0, y: 0, width: 8, height: 8 },
    annotations,
    DEFAULT_IMAGE_ADJUSTMENTS,
  );

  const lit = pixel(ctx, 1, 4);
  assert(lit[0] > 150 && lit[1] < 80, `spotlight altered the lit area: ${lit}`);
  const dimmed = pixel(ctx, 6, 4);
  assert(dimmed[2] < 120 * 0.6, `spotlight did not dim the surroundings: ${dimmed}`);
  assert(dimmed[1] > 60 && dimmed[2] > dimmed[0],
    `highlight covered the base image instead of tinting it: ${dimmed}`);
}

/**
 * 回归真实故障比例：2560×1440 source 的补偿预览是 3413×1920。为控制 smoke
 * 资源使用，这里严格缩小 10 倍为 256×144 与 341.3×192；比率与坐标错误完全相同。
 *
 * preview 与 export 都直接读取真实 Canvas 像素。补偿尺寸只作为“错误 scale”哨兵：
 * canonical preview 必须使用 source 256×144 算 scale=0.5，绝不能使用 341.3×192 算
 * scale≈0.375。边角矢量、中心矢量、blur、mosaic 分别在预览与原尺寸导出中取样。
 */
function verifyCanonicalSourcePixelsAcrossCompensation(): void {
  const sourceWidth = 256;
  const sourceHeight = 144;
  const compensationWidth = 341.3;
  const compensationHeight = 192;
  const cssWidth = 128;
  const cssHeight = 72;
  const canonicalScale = cssWidth / sourceWidth;
  const wrongCompensationScale = cssWidth / compensationWidth;
  assert(Math.abs(canonicalScale - 0.5) < 1e-9, `canonical scale changed: ${canonicalScale}`);
  assert(Math.abs(wrongCompensationScale - 0.375) < 0.001,
    `compensation ratio no longer models 3413/2560: ${wrongCompensationScale}`);

  const source = document.createElement("canvas");
  source.width = sourceWidth;
  source.height = sourceHeight;
  const sourceCtx = context(source);
  for (let y = 0; y < sourceHeight; y += 8) {
    for (let x = 0; x < sourceWidth; x += 8) {
      const even = ((x / 8) + (y / 8)) % 2 === 0;
      sourceCtx.fillStyle = even ? "rgb(235, 45, 35)" : "rgb(25, 75, 215)";
      sourceCtx.fillRect(x, y, 8, 8);
    }
  }
  Object.assign(source, { naturalWidth: sourceWidth, naturalHeight: sourceHeight });

  const effect = { blurRadius: 8, mosaicCell: 12, spotlightDim: 0.55, magnifierZoom: 2 };
  const annotations: Annotation[] = [
    { id: "edge-blur", type: "blur", rect: { x: 8, y: 8, width: 48, height: 40 }, effect },
    { id: "center-mosaic", type: "mosaic", rect: { x: 104, y: 52, width: 48, height: 40 }, effect },
    {
      id: "center-vector",
      type: "pen",
      color: "#ff00ff",
      size: 8,
      points: [{ x: 96, y: 70 }, { x: 160, y: 70 }],
    },
    {
      id: "edge-vector",
      type: "rect",
      color: "#00ff00",
      size: 6,
      rect: { x: 208, y: 108, width: 36, height: 24 },
    },
  ];

  const previewBase = document.createElement("canvas");
  const preview = document.createElement("canvas");
  for (const canvas of [previewBase, preview]) {
    drawScene(
      canvas,
      source as unknown as HTMLImageElement,
      {
        width: cssWidth,
        height: cssHeight,
        fitScale: canonicalScale,
        zoom: 1,
        scale: canonicalScale,
      },
      canvas === preview ? annotations : [],
      null,
      DEFAULT_IMAGE_ADJUSTMENTS,
      null,
    );
  }
  const previewCtx = context(preview);
  const previewBaseCtx = context(previewBase);

  const exportedBase = document.createElement("canvas");
  exportedBase.width = sourceWidth;
  exportedBase.height = sourceHeight;
  const exportedBaseCtx = context(exportedBase);
  renderExport(
    exportedBaseCtx,
    source as unknown as HTMLImageElement,
    { x: 0, y: 0, width: sourceWidth, height: sourceHeight },
    [],
    DEFAULT_IMAGE_ADJUSTMENTS,
  );
  const exported = document.createElement("canvas");
  exported.width = sourceWidth;
  exported.height = sourceHeight;
  const exportedCtx = context(exported);
  renderExport(
    exportedCtx,
    source as unknown as HTMLImageElement,
    { x: 0, y: 0, width: sourceWidth, height: sourceHeight },
    annotations,
    DEFAULT_IMAGE_ADJUSTMENTS,
  );

  const sample = (ctx: CanvasRenderingContext2D, canvas: HTMLCanvasElement, x: number, y: number) =>
    sourcePixel(ctx, canvas, x, y, sourceWidth, sourceHeight);
  const assertEffectRegion = (
    name: string,
    bounds: { x0: number; x1: number; y0: number; y1: number },
  ) => {
    let previewDelta = 0;
    let exportDelta = 0;
    for (let y = bounds.y0; y <= bounds.y1; y += 2) {
      for (let x = bounds.x0; x <= bounds.x1; x += 2) {
        previewDelta = Math.max(previewDelta,
          rgbDistance(sample(previewCtx, preview, x, y), sample(previewBaseCtx, previewBase, x, y)));
        exportDelta = Math.max(exportDelta,
          rgbDistance(sample(exportedCtx, exported, x, y), sample(exportedBaseCtx, exportedBase, x, y)));
      }
    }
    assert(previewDelta > 35, `${name} missing from canonical preview: max delta=${previewDelta}`);
    assert(exportDelta > 35, `${name} missing from source export: max delta=${exportDelta}`);
  };
  // 取效果右/下缘区域：若坐标误乘 2560/3413，这些区域都会落到错误矩形之外。
  assertEffectRegion("edge blur", { x0: 48, x1: 54, y0: 16, y1: 40 });
  assertEffectRegion("center mosaic", { x0: 136, x1: 150, y0: 80, y1: 90 });

  const previewCenterVector = sample(previewCtx, preview, 128, 70);
  const exportCenterVector = sample(exportedCtx, exported, 128, 70);
  assert(previewCenterVector[0] > 180 && previewCenterVector[2] > 180,
    `center vector moved in preview: ${previewCenterVector}`);
  assert(exportCenterVector[0] > 180 && exportCenterVector[2] > 180,
    `center vector moved in export: ${exportCenterVector}`);

  const previewEdgeVector = sample(previewCtx, preview, 208, 120);
  const exportEdgeVector = sample(exportedCtx, exported, 208, 120);
  assert(previewEdgeVector[1] > previewEdgeVector[0] * 1.8,
    `edge vector moved in preview: ${previewEdgeVector}`);
  assert(exportEdgeVector[1] > exportEdgeVector[0] * 1.8,
    `edge vector moved in export: ${exportEdgeVector}`);

  // 若误用 341.3 宽补偿图，边角矩形会落在约 (156,90) source 对应的屏幕位置。
  const wrongX = 208 * wrongCompensationScale / canonicalScale;
  const wrongY = 120 * (cssHeight / compensationHeight) / canonicalScale;
  const wrongLocation = sample(previewCtx, preview, wrongX, wrongY);
  assert(!(wrongLocation[1] > wrongLocation[0] * 1.8),
    `edge vector was multiplied by 3413/2560: ${wrongLocation}`);

  document.documentElement.dataset.canonicalCanvas = "passed";
}

try {
  const source = createSource();
  verifyCropAdjustmentsAndRoundedMask(source);
  verifyVectorAnnotation(source);
  verifyTranslucentAndDimmingEffects(source);
  verifyCanonicalSourcePixelsAcrossCompensation();
  document.documentElement.dataset.canvasExport = "passed";
  document.body.style.background = "#00d000";
} catch (error) {
  document.documentElement.dataset.canvasExport = "failed";
  document.body.title = String(error);
  console.error(String(error));
}
