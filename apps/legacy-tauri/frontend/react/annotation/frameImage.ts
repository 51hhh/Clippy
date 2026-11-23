/**
 * 标注渲染的底图来源。
 *
 * 截图覆盖层、图片编辑器与 Pin 窗口通常使用浏览器原生解码的 `<img>`；截图协议
 * 不可用时会把原始 RGBA 用 `putImageData` 落进 canvas 兜底。两者都是合法的
 * `drawImage` 源，所以渲染层只需要认这个联合类型。
 */
export type FrameImage = HTMLImageElement | HTMLCanvasElement;

/**
 * 底图的像素宽高。
 *
 * `<img>` 用 `naturalWidth`、canvas 用 `width`，用属性存在性区分而不是 `instanceof`：
 * 覆盖层与编辑器都可能拿到别的 realm（iframe / 弹窗）创建的元素，那时 `instanceof` 会失手。
 */
export function frameWidth(image: FrameImage): number {
  return "naturalWidth" in image ? image.naturalWidth : image.width;
}

export function frameHeight(image: FrameImage): number {
  return "naturalHeight" in image ? image.naturalHeight : image.height;
}

/**
 * 把原始 RGBA 字节铺进一块离屏 canvas，作为标注渲染的底图。
 *
 * 约定与后端 `get_capture_frame` 一致：RGBA8、行优先、无 padding。
 */
export function rgbaToFrameCanvas(
  rgba: Uint8ClampedArray<ArrayBuffer>,
  width: number,
  height: number,
): HTMLCanvasElement {
  return paintRgbaFrame(document.createElement("canvas"), rgba, width, height);
}

/** 把原始 RGBA 直接写进指定画布，避免截图首帧额外创建并合成一块全屏画布。 */
export function paintRgbaFrame(
  canvas: HTMLCanvasElement,
  rgba: Uint8ClampedArray<ArrayBuffer>,
  width: number,
  height: number,
): HTMLCanvasElement {
  const expected = width * height * 4;
  if (width < 1 || height < 1 || rgba.length !== expected) {
    throw new Error(`Frame buffer size mismatch: got ${rgba.length}, expected ${expected}`);
  }
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is not available");
  ctx.putImageData(new ImageData(rgba, width, height), 0, 0);
  return canvas;
}
