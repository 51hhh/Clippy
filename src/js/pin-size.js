const MAX_PIN_WIDTH = 900;
const MAX_PIN_HEIGHT = 700;
const MIN_PIN_WIDTH = 180;
const MIN_PIN_HEIGHT = 120;

export function parsePinDimension(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) return null;
  return Math.round(numeric);
}

export function fitPinImageSize(width, height) {
  const safeWidth = Math.max(1, Number(width) || 1);
  const safeHeight = Math.max(1, Number(height) || 1);
  const scale = Math.min(MAX_PIN_WIDTH / safeWidth, MAX_PIN_HEIGHT / safeHeight, 1);
  return {
    width: Math.max(MIN_PIN_WIDTH, Math.round(safeWidth * scale)),
    height: Math.max(MIN_PIN_HEIGHT, Math.round(safeHeight * scale)),
  };
}

export function resolveTempPinBaseSize(naturalWidth, naturalHeight, queryWidth, queryHeight) {
  const width = parsePinDimension(queryWidth);
  const height = parsePinDimension(queryHeight);
  if (width && height) return { width, height };
  return fitPinImageSize(naturalWidth, naturalHeight);
}
