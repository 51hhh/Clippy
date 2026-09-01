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

/**
 * 窗口速选候选区。
 *
 * **数组顺序即堆叠顺序，索引 0 是最上层。** 命中测试必须取第一个包含光标的候选，
 * 这样点在重叠处选到的就是肉眼看到的那个窗口，被完全遮住的窗口自然选不到。
 */
export interface WindowCandidate {
  x: number;
  y: number;
  width: number;
  height: number;
  title: string;
}

/** 截图辅助服务（GNOME Shell 扩展：窗口几何 + 冻结帧）的状态 */
export interface WindowProbeStatus {
  /** 当前桌面用不用得上这个扩展（只有 GNOME Wayland 有意义） */
  supported: boolean;
  installed: boolean;
  /** uuid 已写进 org.gnome.shell enabled-extensions */
  enabled: boolean;
  /** 扩展真的在应答 D-Bus——"功能可用"的唯一判据 */
  active: boolean;
  /** 在应答但版本比内嵌的旧：文件升级过，跑着的仍是上次登录加载的那份 */
  stale: boolean;
  /** 用户是否在系统层面关掉了全部 GNOME 扩展 */
  userExtensionsEnabled: boolean;
}

export interface WindowProbeInstallOutcome {
  /** 为真时要提示用户注销一次后才生效 */
  needsLogout: boolean;
  status: WindowProbeStatus;
}

/**
 * 截图几何诊断报告。
 *
 * **不含任何截图像素，也不含任何窗口标题**（标题会泄露用户正在做什么）。
 * 原样显示给用户看，发不发由用户自己决定——前端绝不自动上传。
 */
export interface CaptureDiagnosticsReport {
  /** 完整报告文本，可直接贴进 issue */
  text: string;
  /** 报告落盘位置；写不进去时为 null，报告本身照样有效 */
  path: string | null;
  /** 可直接存成回归测试 fixture 的 json；拿不到舞台图时为 null */
  fixtureJson: string | null;
}

export interface CaptureOverlayPayload {
  sessionId: string;
  monitorId: number;
  /**
   * 这块显示器在桌面逻辑坐标系里的左上角。覆盖层内的选区坐标是相对自己的，
   * 加上这个偏移才是"屏幕上的哪一块"——贴图靠它贴回原位。
   */
  logicalX: number;
  logicalY: number;
  logicalWidth: number;
  logicalHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  windows: WindowCandidate[];
  /**
   * 这次要不要在覆盖层里提示"窗口速选需要在设置页安装服务"。
   * 后端只置真一次（GNOME Wayland 且扩展没在应答），覆盖层照做，自己不判断桌面环境。
   */
  probeHint: boolean;
}

export interface CaptureSelection {
  sessionId: string;
  monitorId: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 覆盖层里点提交按钮后要做的事。标注在覆盖层内完成，所以没有"转到编辑器"。 */
export type CaptureAction = "copy" | "save" | "pin";

/**
 * 选区在桌面逻辑坐标系里的矩形，与 Rust `pin::PinOrigin` 一一对应。
 * 贴图靠它回到截图时的原位、按原尺寸显示。
 */
export interface CaptureOrigin {
  x: number;
  y: number;
  width: number;
  height: number;
}

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
  /**
   * 压在普通窗口上面（工具条里的图钉）。**默认 false**。
   *
   * 关着的时候贴图就是个普通窗口：谁最后拿到焦点谁在上面。开着的时候进合成器的置顶层，
   * 压在所有普通窗口之上；同时置顶的几张贴图之间仍然按焦点顺序互相遮挡。
   */
  above: boolean;
  canSave: boolean;
  position: { x: number; y: number } | null;
  /**
   * 内容所在那块屏上，一个 CSS 像素等于几个设备像素（合成器报的真实缩放）。
   *
   * `devicePixelRatio` 顶替不了它：GTK3 不支持 `wp_fractional_scale_v1`，分数缩放的桌面上
   * WebKit 一律拿到整数缓冲区缩放 2。判断"屏上一个图片像素是不是正好一个设备像素"
   * 只能靠这个数，见 `react/pin/rendering.ts`。
   */
  deviceScale: number;
  /**
   * 同一块屏上 GTK 报的**整数缓冲区缩放**（分数缩放的桌面上通常是 2）。
   *
   * 后端会在后台把图重新渲染成缓冲区分辨率并预先补偿掉合成器的缩小
   * （`pin/resample.rs`），换进来之后 `pixelWidth == cssWidth * bufferScale`，
   * 前端据此知道这张图是 1:1 搬进缓冲区的、不该再动滤镜。
   */
  bufferScale: number;
}

/**
 * 贴图工具条能待的范围：窗口里"还落在屏幕工作区内"的那块，窗口局部逻辑坐标。
 *
 * 宽或高为 0 表示后端查不到窗口几何（窗口刚关掉、扩展还没认出这个窗口），
 * 前端此时退回整个窗口。**不要拿 `window.innerWidth` 当边界**：贴图窗口的外框恒等于
 * 「内容 + 阴影 + 控件栏」，永远给工具条留够了位置，那样"超出屏幕自动调整"永不触发。
 */
export interface PinToolbarBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * 随保存一起送去的画布工程内容。**原图不在这里**——后端自己从条目取
 * （前端手上那张可能是清晰度补偿版）。
 */
export interface PinCanvasProject {
  /** `Annotation[]` 的原样 JSON。后端不解释它，只搬运进 PNG 的 iTXt 块。 */
  annotations: unknown;
  /** `ImageAdjustments` 的原样 JSON。 */
  adjustments: unknown;
}

/**
 * 从一张 PNG 里读回来的贴图工程（`read_pin_project`）。
 *
 * 拿不到就是 `null`：没有工程块、块坏了、版本比当前应用新——对用户都是同一件事，
 * 这张图能看但不能继续编辑。
 */
export interface PinProject {
  format: string;
  version: number;
  createdAt: number;
  appVersion: string;
  /** 底图：条目原图的 base64 PNG。 */
  sourcePngBase64: string;
  annotations: unknown;
  adjustments: unknown;
}

/** 后台算好的清晰版贴图。见 `pin/commands.rs` 的 `spawn_sharpen`。 */
export interface PinImageSharpened {
  label: string;
  imageBase64: string;
}

/**
 * `update_pin` 的应答：只有这次可能变的那几个字段（Rust 侧 `pin::model::PinState`）。
 *
 * 故意不含 `imageBase64`/`text`：滚轮缩放时每帧都会调一次 `update_pin`，而内容从未变过，
 * 带上图片等于每帧把整张 PNG 重新 base64 编一遍再过一次 IPC。前端把它合并进现有 payload。
 */
export interface PinState {
  label: string;
  contentWidth: number;
  contentHeight: number;
  scale: number;
  opacity: number;
  locked: boolean;
  above: boolean;
  position: { x: number; y: number } | null;
}

export interface PinUpdate {
  scale?: number;
  opacity?: number;
  locked?: boolean;
  above?: boolean;
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
