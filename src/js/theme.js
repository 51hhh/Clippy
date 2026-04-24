/**
 * theme.js — 主题管理模块
 * 从后端配置读取主题，并应用到 <html> 的 data-theme 属性。
 */

import { getConfig } from "./api.js";

/**
 * 初始化主题：读取配置并应用。
 * 若读取失败则保持 HTML 中已设定的默认值（light）。
 */
export async function init() {
  try {
    const config = await getConfig();
    applyTheme(config.theme || "light");
  } catch (err) {
    console.warn("无法加载配置，使用默认主题 light:", err);
    applyTheme("light");
  }
}

/**
 * 将指定主题应用到文档根元素。
 * @param {string} theme — "light" | "dark" | "ocean" | "forest"
 */
export function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}
