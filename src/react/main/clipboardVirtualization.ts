type RowLike = { content_type: string };

// 行高由 components.css 固定。图片要容纳 48px 缩略图和 meta，其余行最多两行文本。
export const TEXT_ROW_HEIGHT = 77;
export const IMAGE_ROW_HEIGHT = 87;
export const LIST_OVERSCAN_PX = 320;

export function clipboardRowHeight(item: RowLike): number {
  return item.content_type === "image" ? IMAGE_ROW_HEIGHT : TEXT_ROW_HEIGHT;
}

/** 每一项顶部的位置；最后一个元素是完整内容高度。 */
export function clipboardRowOffsets(items: readonly RowLike[]): number[] {
  const offsets = new Array<number>(items.length + 1);
  offsets[0] = 0;
  for (let index = 0; index < items.length; index += 1) {
    offsets[index + 1] = offsets[index] + clipboardRowHeight(items[index]);
  }
  return offsets;
}

export type VirtualRange = {
  start: number;
  end: number;
  paddingTop: number;
  paddingBottom: number;
};

/** 找到第一个底边越过 pixel 的行。 */
function firstRowEndingAfter(offsets: readonly number[], pixel: number): number {
  let low = 0;
  let high = Math.max(0, offsets.length - 1);
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (offsets[middle + 1] <= pixel) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** 找到顶部不小于 pixel 的第一个行索引，作为 slice 的排他 end。 */
function firstRowStartingAtOrAfter(offsets: readonly number[], pixel: number): number {
  const count = Math.max(0, offsets.length - 1);
  let low = 0;
  let high = count;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (offsets[middle] < pixel) low = middle + 1;
    else high = middle;
  }
  return low;
}

export function clipboardVisibleRange(
  offsets: readonly number[],
  scrollTop: number,
  viewportHeight: number,
  overscan = LIST_OVERSCAN_PX,
): VirtualRange {
  const count = Math.max(0, offsets.length - 1);
  const totalHeight = offsets[count] ?? 0;
  if (count === 0) return { start: 0, end: 0, paddingTop: 0, paddingBottom: 0 };

  const safeHeight = Math.max(1, viewportHeight);
  const boundedTop = Math.min(
    Math.max(0, scrollTop),
    Math.max(0, totalHeight - safeHeight),
  );
  const startPixel = Math.max(0, boundedTop - Math.max(0, overscan));
  const endPixel = Math.min(totalHeight, boundedTop + safeHeight + Math.max(0, overscan));
  const start = Math.min(count - 1, firstRowEndingAfter(offsets, startPixel));
  const end = Math.max(start + 1, firstRowStartingAtOrAfter(offsets, endPixel));
  return {
    start,
    end: Math.min(count, end),
    paddingTop: offsets[start],
    paddingBottom: totalHeight - offsets[Math.min(count, end)],
  };
}
