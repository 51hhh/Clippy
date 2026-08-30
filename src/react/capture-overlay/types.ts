import type { CaptureTranslationResult } from "../../js/ipc-types.ts";
import type { Tool } from "../annotation/types";

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

/**
 * 覆盖层里的工具。`select` 是默认工具，负责框选/移动/缩放选区；
 * 标注核心的 `crop` 工具在这里没有位置——选区自己就是裁剪框。
 */
export type OverlayTool = "select" | Exclude<Tool, "crop">;

export type CaptureTranslationState =
  | { status: "loading" }
  | { status: "result"; result: CaptureTranslationResult }
  | { status: "error"; message: string };
