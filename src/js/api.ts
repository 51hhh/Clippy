/**
 * api.ts - 公共 IPC facade。
 *
 * 调用方始终从此模块导入；只有 `api/` 下登记的领域模块可以直接访问 Tauri。
 */

export type {
  AppConfig,
  ArchiveExportResult,
  ArchiveImportSummary,
  ArchiveScope,
  CaptureAction,
  CaptureActionResult,
  CaptureDiagnosticsReport,
  CaptureOrigin,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  ClipboardStats,
  ClipItem,
  ContentType,
  InstallType,
  OcrHealthStatus,
  OcrModelIdentity,
  LongshotActivation,
  LongshotAutoCapability,
  LongshotAutoDirection,
  LongshotControllerError,
  LongshotControllerOpenResult,
  LongshotHandle,
  LongshotOutputAction,
  LongshotOutputResult,
  LongshotSnapshot,
  PasteBackend,
  PasteOutcome,
  PastePhase,
  PasteStatus,
  PlatformCapabilities,
  PlatformCapability,
  PlatformInfo,
  PinImageSharpened,
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  PinPayload,
  PinWorkspaceGroup,
  PinWorkspaceStatus,
  PinToolbarBounds,
  PinState,
  PinUpdate,
  ServiceTranslation,
  ShortcutConflict,
  ShortcutRegisterFailure,
  SpokenText,
  TranslationBatch,
  TranslationHistoryEntry,
  TranslationProvider,
  TranslationResult,
  TranslationServiceConfig,
  UrlMeta,
  WindowCandidate,
  WindowProbeInstallOutcome,
  WindowProbeStatus,
} from "./ipc-types.ts";

export type { AppUpdateSnapshot } from "./ipc-types.ts";
export type { DetectedImageCode, ImageCodePoint, ImageCodeScanResponse } from "./api/validators.ts";

export * from "./api/clipboard.ts";
export * from "./api/capture.ts";
export * from "./api/pin.ts";
export * from "./api/viewer.ts";
export * from "./api/settings.ts";
