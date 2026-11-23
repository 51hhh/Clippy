export interface GuideRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

const PARAMS = ["x", "y", "width", "height"] as const;

/** 只接受后端生成、且完整落在当前显示器逻辑 viewport 内的有限矩形。 */
export function parseGuideRect(
  search: string,
  viewportWidth: number,
  viewportHeight: number,
): GuideRect | null {
  if (!Number.isFinite(viewportWidth) || !Number.isFinite(viewportHeight)
    || viewportWidth <= 0 || viewportHeight <= 0) return null;
  const query = new URLSearchParams(search);
  if (!PARAMS.every((key) => query.has(key))) return null;
  const values = Object.fromEntries(PARAMS.map((key) => [key, Number(query.get(key))])) as unknown as GuideRect;
  if (!PARAMS.every((key) => Number.isFinite(values[key]))) return null;
  if (values.x < 0 || values.y < 0 || values.width < 1 || values.height < 1) return null;
  const epsilon = 0.5;
  if (values.x + values.width > viewportWidth + epsilon
    || values.y + values.height > viewportHeight + epsilon) return null;
  return values;
}
