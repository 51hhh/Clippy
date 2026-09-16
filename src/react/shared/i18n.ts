import { getConfig } from "../../js/api.ts";
import { init, t } from "../../i18n/i18n.js";

/** 在 React 窗口首次渲染前同步应用语言，配置不可用时跟随系统。 */
export async function initializeReactI18n(): Promise<void> {
  let language = "auto";
  try {
    language = (await getConfig()).language || "auto";
  } catch {
    // 独立预览或启动早期读取失败时使用浏览器语言。
  }
  init(language);
}

export { t };
