/**
 * 在隔离文本节点中解码 HTML 实体，不把用户输入作为可执行标记插入页面。
 */
export function decodeHtmlEntities(text) {
  if (typeof text !== "string" || !text.includes("&")) return text;

  // 尖括号先转义，确保输入中的标签不会成为解析树的一部分。
  const escapedText = text.replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const parsed = new DOMParser().parseFromString(
    `<textarea>${escapedText}</textarea>`,
    "text/html",
  );
  return parsed.querySelector("textarea")?.value ?? text;
}
