export function formatByteSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export async function loadStats({ getStats, elements }) {
  try {
    const stats = await getStats();
    elements.total.textContent = stats.total;
    elements.favorites.textContent = stats.favorites;
    elements.text.textContent = stats.text_count;
    elements.html.textContent = stats.html_count;
    elements.image.textContent = stats.image_count;
    elements.size.textContent = formatByteSize(stats.db_size);
  } catch (error) {
    console.warn("加载统计失败:", error);
  }
}
