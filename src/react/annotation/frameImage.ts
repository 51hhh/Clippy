/**
 * 标注渲染的底图来源。
 *
 * 截图覆盖层的底图是后端直传的原始 RGBA，用 `putImageData` 落在一块离屏 canvas 上
 * （见 docs/capture-linux.md §3：像素走 PNG + base64 要在两头各编解码一次，
 * 全屏帧实测占掉覆盖层出现前的一半时间）。图片编辑器与 Pin 窗口的底图仍然是
 * `<img>`。两者都是合法的 `drawImage` 源，所以渲染层只需要认这个联合类型。
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
  const expected = width * height * 4;
  if (width < 1 || height < 1 || rgba.length !== expected) {
    throw new Error(`Frame buffer size mismatch: got ${rgba.length}, expected ${expected}`);
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is not available");
  ctx.putImageData(new ImageData(rgba, width, height), 0, 0);
  return canvas;
}
