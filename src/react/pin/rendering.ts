/**
 * 贴图该用哪种缩放滤镜——以及为什么"最近邻"在这里反而是最清晰的那个。
 *
 * ## 为什么贴出来会糊
 *
 * GTK3 不支持 `wp_fractional_scale_v1`，所以分数缩放的桌面上 Mutter 只能给 WebKit 一个
 * **整数缓冲区缩放 2**，而显示器的真实缩放是 1.5（或 4/3）。于是一张按原物理尺寸显示的图
 * 要走两道重采样：
 *
 * ```text
 * 源像素 --WebKit 放大 k = 2 / 1.5 = 4/3--> 缓冲区 --合成器缩小 1.5/2 = 3/4--> 屏幕
 * ```
 *
 * 两次相乘正好是 1（屏上就是原尺寸），但中间那趟 4/3 上采样把细节抹平了，抹掉的东西
 * 后面那趟缩小拿不回来——这就是"选区里清楚、pin 出来糊"。窗口这边无解：k 由合成器定，
 * 客户端改不了；把图按 CSS = 像素/2 显示能得到 1:1 的缓冲区，但那样贴图只有 0.75 倍大，
 * 不再是原尺寸。
 *
 * ## 能改的是滤镜
 *
 * 缩放比例定死了，但**用什么滤镜搬进缓冲区**是 CSS 说得上话的。关键在于：
 * 最近邻放大 4/3（每三列复制一列）之后再被合成器按 3/4 平均缩回去，两步几乎互为逆运算，
 * 原始像素值基本原样落回屏幕；而任何平滑滤镜都是在第一步就把信息糊掉，第二步只能糊上加糊。
 *
 * 本机 HDMI-1（3840x2160 原生 / 2560x1440 逻辑，缩放 1.5）实测，把成像结果与源图逐像素比：
 *
 * | 做法 | 1 px 棋盘 | 真实截图内容 |
 * |---|---|---|
 * | 默认（平滑） | PSNR 11.78 dB | 30.06 dB |
 * | canvas 按 dpr 开 `imageSmoothingQuality:"high"` 重采样 | 13.32 dB | — |
 * | 源图先用 Lanczos 放大到缓冲区尺寸 | — | — |
 * | **`image-rendering: pixelated`** | **33.74 dB** | **33.73 dB** |
 *
 * `pixelated` / `crisp-edges` / `-webkit-optimize-contrast` 在 WebKitGTK 里是同一条路
 * （三者实测差 0.01 dB），所以用标准关键字。canvas 那条路也能赢默认，但要多占一份
 * 按缓冲区尺寸算的位图（全屏贴图约 33 MB），而效果还不如一行 CSS。
 *
 * ## 但只在"屏上一个图片像素正好一个设备像素"时成立
 *
 * 最近邻好用的前提是那两步互为逆运算。一旦贴图不是原尺寸，第一步就是真的在重采样，
 * 最近邻会直接丢像素/出锯齿。同一台机器上的对照（参考值是 Lanczos 缩放的理想结果）：
 *
 * | 屏上倍数 | 默认平滑 | `pixelated` |
 * |---|---|---|
 * | 0.6 | 30.84 dB | 26.37 dB |
 * | 1.0 | 30.06 dB | **33.73 dB** |
 * | 1.2 | 30.72 dB | 27.86 dB |
 * | 1.6 | 31.78 dB | 28.63 dB |
 *
 * 所以判据不能写成 `scale === 1`：全屏截图贴出来时内容会被 `origin_content_size` 按工作区
 * 缩小，而 `scale` 仍然是 1，那种情况按最近邻画就是上表 0.6 那一行。必须直接比
 * **显示尺寸与图片像素**，这也顺带覆盖了 X11（真实缩放 = 1，两者本来就相等）。
 *
 * ## 上面这些都只是"没有更好办法时"的选择
 *
 * `pixelated` 的 33.95 dB 比默认的 30.28 dB 好，但仍然看得出糊——因为它没有改变"信息在
 * 4/3 上采样里丢掉"这件事，只是丢得整齐一点。真正的解法在后端：把图**预先渲染成缓冲区
 * 分辨率**，并按合成器那一步的实测核做反投影补偿，实测 43.45 dB（`pin/resample.rs`）。
 * 那份图算起来要几百毫秒，所以贴图先用原图上屏、算完再换进来。
 *
 * 换进来之后图片像素数正好等于 `cssWidth * bufferScale`，也就是 1:1 搬进缓冲区——
 * 这时候**任何滤镜都不该介入**（缩放比为 1，`auto` 就是恒等），所以判据要认这一条，
 * 而不是让它掉进"不是像素对齐"的分支里靠巧合拿到 `auto`。
 */

export interface PinImageMetrics {
  /** 内容区的 CSS 宽高（= `contentWidth * scale`）。 */
  cssWidth: number;
  cssHeight: number;
  /** 图片自己的像素尺寸（`naturalWidth` / `naturalHeight`）。 */
  pixelWidth: number;
  pixelHeight: number;
  /** 这块屏上一个 CSS 像素等于几个设备像素，由后端查合成器得到。 */
  deviceScale: number;
  /**
   * WebKit 缓冲区的整数缩放（GTK 报的那个）。图片是后端补偿过的版本时，
   * `pixelWidth === cssWidth * bufferScale`，也就是 1:1 搬进缓冲区。
   */
  bufferScale: number;
}

/** 图片是不是正好 1:1 落进 WebKit 缓冲区（= 后端补偿过的那份）。 */
export function isBufferExact(metrics: PinImageMetrics): boolean {
  const { cssWidth, cssHeight, pixelWidth, pixelHeight, bufferScale } = metrics;
  if (![cssWidth, cssHeight, pixelWidth, pixelHeight, bufferScale].every(Number.isFinite)) {
    return false;
  }
  if (bufferScale <= 0 || pixelWidth <= 0 || pixelHeight <= 0) return false;
  return (
    Math.abs(cssWidth * bufferScale - pixelWidth) <= 1
    && Math.abs(cssHeight * bufferScale - pixelHeight) <= 1
  );
}

/**
 * 屏上的成像尺寸是不是正好等于图片自己的像素数。
 *
 * 容差 1 个设备像素：窗口外框尺寸要取整到整数逻辑像素，内容尺寸与图片像素之间因此
 * 常有不到一像素的零头，那不影响结论。
 */
export function isPixelExact(metrics: PinImageMetrics): boolean {
  const { cssWidth, cssHeight, pixelWidth, pixelHeight, deviceScale } = metrics;
  if (![cssWidth, cssHeight, pixelWidth, pixelHeight, deviceScale].every(Number.isFinite)) {
    return false;
  }
  if (deviceScale <= 0 || pixelWidth <= 0 || pixelHeight <= 0) return false;
  return (
    Math.abs(cssWidth * deviceScale - pixelWidth) <= 1
    && Math.abs(cssHeight * deviceScale - pixelHeight) <= 1
  );
}

/** 交给 CSS `image-rendering` 的值。 */
export type PinImageRendering = "pixelated" | "auto";

/**
 * 上面那套结论的唯一出口。
 *
 * 两个分支的含义不一样，但结果碰巧一致，所以不写成三个分支：
 * - 像素对齐（没有补偿、屏上一个图片像素一个设备像素）→ `pixelated`，实测 +3.7 dB。
 * - 补偿过的图（[`isBufferExact`]，1:1 进缓冲区）→ 缩放比为 1，`auto` 就是恒等变换。
 * - 其余（缩放过、被工作区缩小过）→ 真的在重采样，`auto` 的平滑最好。
 *
 * 两个判据不会同时成立：只有 `bufferScale !== deviceScale` 时后端才补偿，
 * 那时 `cssWidth * deviceScale` 与 `cssWidth * bufferScale` 差着几百像素。
 * [`isBufferExact`] 把这条不变量摆在明面上，由测试锁住。
 */
export function pinImageRendering(metrics: PinImageMetrics): PinImageRendering {
  return isPixelExact(metrics) ? "pixelated" : "auto";
}
