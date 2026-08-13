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
  translation_provider: string;
  translation_endpoint: string;
  translation_model: string;
  translation_source_language: string;
  translation_target_language: string;
  main_window_position: { x: number; y: number } | null;
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

export type TranslationProvider = "libretranslate" | "openai_compatible";

export interface TranslationResult {
  request_id: number;
  provider: TranslationProvider;
  translated_text: string;
  detected_source_language: string | null;
}

export interface CaptureTranslationResult {
  requestId: number;
  provider: TranslationProvider;
  sourceText: string;
  translatedText: string;
  detectedSourceLanguage: string | null;
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
