/**
 * 剪贴板行的纯展示格式化函数。
 *
 * 这里故意不提供"类型"格式化：列表行只显示大小和时间。内容类型由
 * `preview/classify.js` 一处判定，只写在右侧预览面板的 badge 上——后端
 * `content_type` 只有 text/html/image 三档，和按内容嗅探出的 YAML/JWT
 * 是两套标准，两边同时显示就会出现"主栏 HTML、侧栏 YAML"这种自相矛盾。
 */

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
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
