/**
 * i18n.js — 轻量国际化模块
 * 翻译数据直接内嵌，无需 fetch 外部 JSON 文件。
 */

const TRANSLATIONS = {
  en: {
    "tabs.all": "All",
    "tabs.favorites": "Favorites",
    "empty.text": "No clipboard items yet",
    "empty.favorites": "No favorites yet",
    "action.favorite": "Favorite",
    "action.unfavorite": "Unfavorite",
    "action.copy": "Copy",
    "action.delete": "Delete",
    "action.confirm": "Confirm?",
    "action.pin": "Pin to Desktop",
    "action.ocr": "OCR",
    "action.ocrProcessing": "Recognizing...",
    "action.ocrFailed": "OCR Failed",
    "action.ocrUnavailable": "OCR unavailable: install tesseract-ocr",
    "action.ocrEmpty": "No text recognized",
    "action.ocrCopyHint": "Ctrl+C to copy selected text",
    "action.more": "More actions",
    "search.placeholder": "Search clipboard history...",
    "search.escHint": "Esc",
    "time.justNow": "just now",
    "time.minutesAgo": "{n} min ago",
    "time.hoursAgo": "{n} hr ago",
    "time.yesterday": "yesterday",
    "time.daysAgo": "{n} days ago",
    "settings.title": "Settings",
    "settings.shortcut.label": "Global Shortcut",
    "settings.shortcut.placeholder": "Click Record to set...",
    "settings.shortcut.record": "Record",
    "settings.shortcut.stop": "Stop",
    "settings.shortcut.reset": "Reset",
    "settings.shortcut.hint": 'Click "Record" then press your desired key combination',
    "settings.shortcut.conflict": "This shortcut may conflict with an existing shortcut",
    "settings.shortcut.recording": "Press keys...",
    "settings.pinShortcut.label": "Pin Shortcut",
    "settings.pinShortcut.placeholder": "Click Record to set...",
    "settings.pinShortcut.hint": "Shortcut to pin the focused clip to desktop",
    "settings.theme.label": "Theme",
    "settings.theme.light": "Linen",
    "settings.theme.dark": "Graphite",
    "settings.theme.nord": "Nord",
    "settings.theme.solarizedLight": "Solarized",
    "settings.theme.rose": "Rose",
    "settings.theme.midnight": "Midnight",
    "settings.history.label": "History Limit",
    "settings.history.hint": "0 = unlimited. Favorites are never deleted.",
    "settings.language.label": "Language",
    "settings.language.auto": "Auto",
    "settings.language.en": "English",
    "settings.language.zhCN": "中文",
    "settings.autostart.label": "Launch at Login",
    "settings.autostart.hint": "Automatically start Clippy when you log in",
    "settings.ocr.label": "OCR Result",
    "settings.ocr.preview": "Show in Preview",
    "settings.ocr.clipboard": "Copy to Clipboard",
    "settings.ocr.hint": "How to handle OCR recognition results",
    "settings.save": "Save",
    "settings.cancel": "Cancel",
    "settings.saved": "Settings saved!",
    "settings.saveFailed": "Save failed: {error}",
    "settings.about.label": "About",
    "settings.about.checkUpdate": "Check for updates",
    "settings.about.upToDate": "Already up to date",
    "settings.about.checkFailed": "Check failed",
    "settings.about.updateAvailable": "New version v{version} available",
    "update.title": "Update Available",
    "update.downloading": "Downloading...",
    "update.fallbackTitle": "Manual Download Required",
    "update.fallbackBody": "Automatic update is not supported for deb packages. Please download the latest version manually.",
    "update.skip": "Skip this version",
    "update.later": "Later",
    "update.install": "Install",
    "update.close": "Close",
    "update.download": "Download",
    "preview.richText": "[Rich Text]",
    "preview.imageLoadFailed": "Image load failed",
  },
  "zh-CN": {
    "tabs.all": "全部",
    "tabs.favorites": "收藏",
    "empty.text": "暂无剪贴板记录",
    "empty.favorites": "暂无收藏",
    "action.favorite": "收藏",
    "action.unfavorite": "取消收藏",
    "action.copy": "复制",
    "action.delete": "删除",
    "action.confirm": "确认?",
    "action.pin": "钉到桌面",
    "action.ocr": "文字识别",
    "action.ocrProcessing": "识别中...",
    "action.ocrFailed": "识别失败",
    "action.ocrUnavailable": "OCR 不可用：请运行 sudo apt install tesseract-ocr tesseract-ocr-chi-sim",
    "action.ocrEmpty": "未识别到文字",
    "action.ocrCopyHint": "Ctrl+C 复制选中文字",
    "action.more": "更多操作",
    "search.placeholder": "搜索剪贴板历史...",
    "search.escHint": "Esc",
    "time.justNow": "刚刚",
    "time.minutesAgo": "{n} 分钟前",
    "time.hoursAgo": "{n} 小时前",
    "time.yesterday": "昨天",
    "time.daysAgo": "{n} 天前",
    "settings.title": "设置",
    "settings.shortcut.label": "全局快捷键",
    "settings.shortcut.placeholder": "点击录制以设置...",
    "settings.shortcut.record": "录制",
    "settings.shortcut.stop": "停止",
    "settings.shortcut.reset": "重置",
    "settings.shortcut.hint": "点击「录制」后按下想要的快捷键组合",
    "settings.shortcut.conflict": "此快捷键可能与已有快捷键冲突",
    "settings.shortcut.recording": "请按键...",
    "settings.pinShortcut.label": "钉图快捷键",
    "settings.pinShortcut.placeholder": "点击录制以设置...",
    "settings.pinShortcut.hint": "将焦点条目钉到桌面的快捷键",
    "settings.theme.label": "主题",
    "settings.theme.light": "亚麻",
    "settings.theme.dark": "石墨",
    "settings.theme.nord": "极地",
    "settings.theme.solarizedLight": "晒纸",
    "settings.theme.rose": "玫瑰",
    "settings.theme.midnight": "深夜",
    "settings.history.label": "历史上限",
    "settings.history.hint": "0 = 不限。收藏条目不受清理影响。",
    "settings.language.label": "语言",
    "settings.language.auto": "跟随系统",
    "settings.language.en": "English",
    "settings.language.zhCN": "中文",
    "settings.autostart.label": "开机自启动",
    "settings.autostart.hint": "登录时自动启动 Clippy",
    "settings.ocr.label": "OCR 结果处理",
    "settings.ocr.preview": "显示在预览面板",
    "settings.ocr.clipboard": "复制到剪贴板",
    "settings.ocr.hint": "OCR 识别结果的处理方式",
    "settings.save": "保存",
    "settings.cancel": "取消",
    "settings.saved": "设置已保存！",
    "settings.saveFailed": "保存失败：{error}",
    "settings.about.label": "关于",
    "settings.about.checkUpdate": "检查更新",
    "settings.about.upToDate": "已是最新版本",
    "settings.about.checkFailed": "检查失败",
    "settings.about.updateAvailable": "发现新版本 v{version}",
    "update.title": "发现新版本",
    "update.downloading": "正在下载...",
    "update.fallbackTitle": "需要手动下载",
    "update.fallbackBody": "deb 安装包不支持自动更新，请手动下载最新版本。",
    "update.skip": "跳过此版本",
    "update.later": "稍后提醒",
    "update.install": "立即安装",
    "update.close": "关闭",
    "update.download": "前往下载",
    "preview.richText": "[富文本]",
    "preview.imageLoadFailed": "图片加载失败",
  },
};

const SUPPORTED_LOCALES = ["en", "zh-CN"];
const FALLBACK_LOCALE = "en";

let _currentLocale = FALLBACK_LOCALE;

/**
 * 初始化 i18n：检测语言 → 应用到 DOM。
 * @param {string} configLanguage — AppConfig.language 值（"auto" | "en" | "zh-CN"）
 */
export function init(configLanguage) {
  _currentLocale = resolveLocale(configLanguage);
  applyToDOM();
}

/**
 * 程序式翻译。
 * @param {string} key — 翻译键（如 "time.minutesAgo"）
 * @param {Record<string, string|number>} [params] — 插值参数（如 {n: 5}）
 * @returns {string}
 */
export function t(key, params) {
  const dict = TRANSLATIONS[_currentLocale] || TRANSLATIONS[FALLBACK_LOCALE];
  let text = dict[key];
  if (text === undefined) {
    text = TRANSLATIONS[FALLBACK_LOCALE][key] || key;
  }
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      text = text.replace(`{${k}}`, String(v));
    }
  }
  return text;
}

/** 返回当前生效的语言代码。 */
export function currentLocale() {
  return _currentLocale;
}

/** 扫描所有 [data-i18n] 元素并替换文本/属性。 */
export function applyToDOM() {
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n;
    const attr = el.dataset.i18nAttr;
    const translated = t(key);
    if (translated === key) return;
    if (attr) {
      el.setAttribute(attr, translated);
    } else {
      el.textContent = translated;
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
  const browserLang = navigator.language || "en";
  if (browserLang.startsWith("zh")) return "zh-CN";
  return FALLBACK_LOCALE;
}
