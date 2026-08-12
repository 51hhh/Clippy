/** 剪贴板行的纯展示格式化函数。 */

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatType(type) {
  if (!type || type === "text") return "Text";
  if (type === "html") return "HTML";
  if (type === "image") return "Image";
  return type;
}

/**
 * @param {number} timestampSeconds Unix 秒时间戳
 * @param {{ now?: number, translate: (key: string, params?: object) => string }} options
 */
export function formatRelativeTime(timestampSeconds, { now = Date.now(), translate }) {
  const elapsedMinutes = Math.floor((now - timestampSeconds * 1000) / 60000);
  if (elapsedMinutes < 1) return translate("time.justNow");
  if (elapsedMinutes < 60) return translate("time.minutesAgo", { n: elapsedMinutes });

  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return translate("time.hoursAgo", { n: elapsedHours });

  const elapsedDays = Math.floor(elapsedHours / 24);
  if (elapsedDays === 1) return translate("time.yesterday");
  return translate("time.daysAgo", { n: elapsedDays });
}
