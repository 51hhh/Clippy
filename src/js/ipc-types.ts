/**
 * Rust serde 与前端之间的共享 IPC 数据合同。
 *
 * 字段命名严格跟随对应 Rust 类型的序列化结果：未声明 rename_all 的
 * 结构保持 snake_case，截图覆盖层和贴图结构使用 camelCase。
 */

export type ContentType = "text" | "html" | "image";

export interface ClipItem {
  id: number;
  content_type: ContentType;
  text_content: string | null;
  html_content: string | null;
  image_data: number[] | null;
  content_hash: string;
  is_favorite: boolean;
  is_sensitive: boolean;
  created_at: number;
  byte_size: number;
}

/** 单个翻译服务的配置；空字符串表示沿用该服务的内置默认值 */
export interface TranslationServiceConfig {
  provider: TranslationProvider;
  enabled: boolean;
  endpoint: string;
  model: string;
  /** Azure 资源区域，仅 Bing 官方 API 使用 */
  region: string;
  /** GCP 项目 ID，仅 Google Cloud v3 使用 */
  project: string;
}

export interface AppConfig {
  version: number;
  max_history: number;
  storage_mode: string;
  global_shortcut: string;
  pin_shortcut: string;
  capture_shortcut: string;
  theme: string;
  language: string;
  delete_confirm_ms: number;
  ocr_result_mode: string;
  ocr_enabled: boolean;
  tmux_capture: boolean;
  auto_paste: boolean;
  translation_services: TranslationServiceConfig[];
  translation_source_language: string;
  translation_target_language: string;
  /** 备选目标语言，按优先级排列；留空表示沿用目标/源语言这一对 */
  preferred_languages: string[];
  main_window_position: { x: number; y: number } | null;
  /** 截图/Pin 保存目录，支持 `~` 开头；留空表示内置默认目录 */
  screenshot_save_dir: string;
  /** 保存文件名模板（`{prefix}` `{date}` `{time}` `{unix}` `{seq}`）；留空表示内置默认 */
  screenshot_filename_template: string;
}

/** 快捷键占用检测结果（`check_shortcut_conflict`） */
export interface ShortcutConflict {
  /** 明确检测到冲突 */
  conflicted: boolean;
  /** `desktop` = 桌面已有绑定，`clippy` = 本应用当前已注册，null = 无冲突 */
  source: "desktop" | "clippy" | null;
  /** 占用者标识（gsettings key 或自定义快捷键名），只用于提示与日志 */
  owner: string | null;
  /** 本会话能否枚举桌面绑定；false 表示查不出来，不等于没有冲突 */
  enumerable: boolean;
}

/** 快捷键注册失败记录（`shortcut-register-failed` 事件与 `get_shortcut_failures`） */
export interface ShortcutRegisterFailure {
  /** 哪个动作没绑上 */
  action: "global" | "pin" | "capture";
  shortcut: string;
  /** 会话类型，决定提示里给出的处置建议 */
  session: "wayland" | "x11";
  /** 底层原因（中文日志文案，只作为补充信息展示） */
  reason: string;
}

export type PasteBackend = "x11" | "wayland_portal" | "copy_only";

export type PastePhase =
  | "ready"
  | "permission_required"
  | "initializing"
  | "denied"
  | "unavailable";

export interface PasteStatus {
  backend: PasteBackend;
  phase: PastePhase;
  auto_paste_enabled: boolean;
  can_request_permission: boolean;
  detail: string | null;
}

export interface PasteOutcome {
  copied: boolean;
  pasted: boolean;
  backend: PasteBackend;
  detail: string | null;
}

export interface CapturedScreenshot {
  pngBase64: string;
  width: number;
  height: number;
  generation: number;
}

export interface WindowCandidate {
  x: number;
  y: number;
  width: number;
  height: number;
  title: string;
}

export interface CaptureOverlayPayload {
  sessionId: string;
  monitorId: number;
  pngBase64: string;
  logicalWidth: number;
  logicalHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  windows: WindowCandidate[];
}

export interface CaptureSelection {
  sessionId: string;
  monitorId: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export type CaptureAction = "copy" | "save" | "pin" | "edit";

export interface CaptureActionResult {
  action: CaptureAction;
  path: string | null;
  pinLabel: string | null;
}

/** 与 Rust `TranslationProvider` 的 serde 名一一对应，字符串值是稳定合同 */
export type TranslationProvider =
  | "libretranslate"
  | "openai_compatible"
  | "deepl"
  | "google"
  | "bing"
  | "youdao";

export interface TranslationResult {
  request_id: number;
  provider: TranslationProvider;
  translated_text: string;
  detected_source_language: string | null;
  /** 实际使用的目标语言，可能因自动换向与设置里的目标语言不同 */
  target_language: string;
}

/**
 * 并行翻译中单个服务的结果。与 Rust `ServiceTranslation` 的 `#[serde(tag = "status")]`
 * 一一对应：失败作为数据返回，前端据此只重试出错的那个服务。
 */
export type ServiceTranslation =
  | {
      status: "ok";
      provider: TranslationProvider;
      translated_text: string;
      detected_source_language: string | null;
      /** 实际使用的目标语言，可能因自动换向与设置里的目标语言不同 */
      target_language: string;
    }
  | {
      status: "error";
      provider: TranslationProvider;
      /** TranslationError::code() 的稳定值 */
      code: string;
    };

export interface TranslationBatch {
  request_id: number;
  /** 顺序与配置里的服务顺序一致 */
  services: ServiceTranslation[];
}

/**
 * 一段可播放的音频。前端拼成 `data:{mime_type};base64,{audio_base64}` 播放，
 * 远端请求由 Rust 完成，webview 不直接访问第三方主机。
 */
export interface SpokenText {
  mime_type: string;
  audio_base64: string;
}

/** 一条落库的翻译记录。`clip_id` 为 0 表示不来自剪贴板条目（选区翻译或临时文本） */
export interface TranslationHistoryEntry {
  id: number;
  clip_id: number;
  provider: TranslationProvider;
  source_language: string;
  target_language: string;
  source_text: string;
  translated_text: string;
  created_at: number;
}

export interface CaptureTranslationResult {
  requestId: number;
  provider: TranslationProvider;
  sourceText: string;
  translatedText: string;
  detectedSourceLanguage: string | null;
  /** 实际使用的目标语言，可能因自动换向与设置里的目标语言不同 */
  targetLanguage: string;
}

export interface PinPayload {
  label: string;
  kind: "image" | "text";
  text: string | null;
  imageBase64: string | null;
  contentWidth: number;
  contentHeight: number;
  scale: number;
  opacity: number;
  locked: boolean;
  canSave: boolean;
  canEdit: boolean;
  position: { x: number; y: number } | null;
}

export interface PinUpdate {
  scale?: number;
  opacity?: number;
  locked?: boolean;
}

export interface ClipboardStats {
  total: number;
  favorites: number;
  text_count: number;
  html_count: number;
  image_count: number;
  sensitive_count: number;
  total_bytes: number;
  db_size: number;
}

export interface UrlMeta {
  url: string;
  title: string | null;
  description: string | null;
  favicon: string | null;
  site_name: string | null;
}

export type InstallType = "appimage" | "deb";
