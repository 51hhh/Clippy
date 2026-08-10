import type { CaptureTranslationResult } from "../../js/ipc-types.ts";

export type {
  CaptureAction,
  CaptureOverlayPayload,
  CaptureSelection,
  CaptureTranslationResult,
  TranslationProvider,
  WindowCandidate,
} from "../../js/ipc-types.ts";

export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; width: number; height: number };
export type ResizeHandle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

export type CaptureTranslationState =
  | { status: "loading" }
  | { status: "result"; result: CaptureTranslationResult }
  | { status: "error"; message: string };
