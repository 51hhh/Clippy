/**
 * i18n.js — 轻量国际化模块
 */

import en from "./en.json";
import zhCN from "./zh-CN.json";

const TRANSLATIONS = { en, "zh-CN": zhCN };
const SUPPORTED_LOCALES = ["en", "zh-CN"];
const FALLBACK_LOCALE = "en";

let _currentLocale = FALLBACK_LOCALE;

/**
 * 初始化 i18n：检测语言并应用到 DOM。
 * @param {string} configLanguage AppConfig.language 值（"auto" | "en" | "zh-CN"）
 */
export function init(configLanguage) {
  _currentLocale = resolveLocale(configLanguage);
  applyToDOM();
}

/**
 * 程序式翻译。
 * @param {string} key 翻译键（如 "time.minutesAgo"）
 * @param {Record<string, string|number>} [params] 插值参数（如 {n: 5}）
 * @returns {string}
 */
export function t(key, params) {
  const dict = TRANSLATIONS[_currentLocale] || TRANSLATIONS[FALLBACK_LOCALE];
  let translated = dict[key];
  if (translated === undefined) {
    translated = TRANSLATIONS[FALLBACK_LOCALE][key] || key;
  }
  if (params) {
    for (const [name, value] of Object.entries(params)) {
      translated = translated.replace(`{${name}}`, String(value));
    }
  }
  return translated;
}

/** 返回当前生效的语言代码。 */
export function currentLocale() {
  return _currentLocale;
}

/** 扫描所有 [data-i18n] 元素并替换文本或属性。 */
export function applyToDOM() {
  document.querySelectorAll("[data-i18n]").forEach((element) => {
    const key = element.dataset.i18n;
    const attribute = element.dataset.i18nAttr;
    const translated = t(key);
    if (translated === key) return;
    if (attribute) {
      element.setAttribute(attribute, translated);
    } else {
      element.textContent = translated;
    }
  });
}

/**
 * 将配置语言值解析为实际 locale 代码。
 * @param {string} configLanguage
 * @returns {string}
 */
function resolveLocale(configLanguage) {
  if (configLanguage && configLanguage !== "auto") {
    return SUPPORTED_LOCALES.includes(configLanguage) ? configLanguage : FALLBACK_LOCALE;
  }
  const browserLanguage = navigator.language || "en";
  if (browserLanguage.startsWith("zh")) return "zh-CN";
  return FALLBACK_LOCALE;
}
