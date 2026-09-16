/**
 * theme.js — 主题管理
 */

import { getConfig } from "./api.ts";

export async function init() {
  try {
    const config = await getConfig();
    applyTheme(config.theme || "light");
  } catch (err) {
    console.warn("无法加载配置，使用默认主题:", err);
    applyTheme("light");
  }
}

export function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}
