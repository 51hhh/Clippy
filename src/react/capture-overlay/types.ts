export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; width: number; height: number };

export type WindowCandidate = Rect & { title: string };

export type CaptureOverlayPayload = {
  sessionId: string;
  monitorId: number;
  pngBase64: string;
  logicalWidth: number;
  logicalHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  windows: WindowCandidate[];
};

export type CaptureSelection = Rect & {
  sessionId: string;
  monitorId: number;
};

export type CaptureAction = "copy" | "save" | "pin" | "edit";
export type ResizeHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

export type TranslationProvider = "libretranslate" | "openai_compatible";

export type CaptureTranslationResult = {
  requestId: number;
  provider: TranslationProvider;
  sourceText: string;
  translatedText: string;
  detectedSourceLanguage: string | null;
};

export type CaptureTranslationState =
  | { status: "loading" }
  | { status: "result"; result: CaptureTranslationResult }
  | { status: "error"; message: string };
