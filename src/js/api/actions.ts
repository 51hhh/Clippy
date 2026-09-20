import { invoke } from "@tauri-apps/api/core";
import type { StructuredOcr, TranslationProvider, TranslationResult } from "../ipc-types.ts";
import { checkedStructuredOcr, parseImageCodeScanResponse } from "./validators.ts";
import type { ImageCodeScanResponse } from "./validators.ts";

export type ActionId =
  | "capture.start"
  | "image.ocr"
  | "image.pin"
  | "image.save"
  | "image.scan_codes"
  | "text.copy"
  | "text.translate";

export type ActionValueKind =
  | "unit"
  | "owned_image"
  | "text"
  | "translation_request"
  | "capture_session"
  | "recognized_text"
  | "detected_codes"
  | "translated_text"
  | "saved_path"
  | "window_handle";

export type ActionPermission =
  | "screen.capture"
  | "image.local_analysis"
  | "translation.network"
  | "clipboard.write"
  | "file.write"
  | "window.create";

export type ActionPlatform = "linux" | "windows" | "macos";

export interface ActionDescriptor {
  id: ActionId;
  input: ActionValueKind;
  output: ActionValueKind;
  permissions: ActionPermission[];
  cancellable: boolean;
  platforms: ActionPlatform[];
}

export interface ActionHandle {
  requestSlot: string;
  generation: number;
}

export interface ActionInputById {
  "capture.start": Record<string, never>;
  "image.ocr": { sourceId: string; sourceVersion: number };
  "image.pin": { sourceId: string; sourceVersion: number };
  "image.save": { sourceId: string; sourceVersion: number };
  "image.scan_codes": { sourceId: string; sourceVersion: number };
  "text.copy": { text: string };
  "text.translate": { text: string; sourceLanguage?: string; targetLanguage: string };
}

export interface ActionOutputById {
  "capture.start": { type: "capture_session"; value: string };
  "image.ocr": { type: "recognized_text"; value: StructuredOcr };
  "image.pin": { type: "window_handle"; value: string };
  "image.save": { type: "saved_path"; value: string };
  "image.scan_codes": { type: "detected_codes"; value: ImageCodeScanResponse };
  "text.copy": { type: "unit" };
  "text.translate": { type: "translated_text"; value: TranslationResult };
}

export interface ActionReply<K extends ActionId> {
  handle: ActionHandle;
  output: ActionOutputById[K];
}

const PLATFORMS: ActionPlatform[] = ["linux", "windows", "macos"];
const CATALOG: Record<ActionId, Omit<ActionDescriptor, "id" | "platforms">> = {
  "capture.start": { input: "unit", output: "capture_session", permissions: ["screen.capture"], cancellable: false },
  "image.ocr": { input: "owned_image", output: "recognized_text", permissions: ["image.local_analysis"], cancellable: true },
  "image.pin": { input: "owned_image", output: "window_handle", permissions: ["window.create"], cancellable: false },
  "image.save": { input: "owned_image", output: "saved_path", permissions: ["file.write"], cancellable: false },
  "image.scan_codes": { input: "owned_image", output: "detected_codes", permissions: ["image.local_analysis"], cancellable: true },
  "text.copy": { input: "text", output: "unit", permissions: ["clipboard.write"], cancellable: false },
  "text.translate": { input: "translation_request", output: "translated_text", permissions: ["translation.network"], cancellable: true },
};

const textEncoder = new TextEncoder();

function isRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function hasExactKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every(key => Object.hasOwn(value, key));
}

function equalStrings(value: unknown, expected: readonly string[]): boolean {
  return Array.isArray(value)
    && value.length === expected.length
    && value.every((entry, index) => entry === expected[index]);
}

function checkedDescriptor(value: unknown): ActionDescriptor {
  if (!isRecord(value) || !hasExactKeys(value, ["id", "input", "output", "permissions", "cancellable", "platforms"])
    || typeof value.id !== "string" || !Object.hasOwn(CATALOG, value.id)) {
    throw new Error("action.invalid_descriptor");
  }
  const id = value.id as ActionId;
  const expected = CATALOG[id];
  if (value.input !== expected.input || value.output !== expected.output
    || value.cancellable !== expected.cancellable
    || !equalStrings(value.permissions, expected.permissions)
    || !equalStrings(value.platforms, PLATFORMS)) {
    throw new Error("action.invalid_descriptor");
  }
  return value as unknown as ActionDescriptor;
}

function checkedRequestSlot(value: unknown): string {
  if (typeof value !== "string" || textEncoder.encode(value).byteLength > 96
    || !/^[A-Za-z0-9_.-]+$/.test(value)) throw new Error("action.invalid_request_slot");
  return value;
}

function checkedActionId(value: unknown): ActionId {
  if (typeof value !== "string" || !Object.hasOwn(CATALOG, value)) throw new Error("action.invalid_id");
  return value as ActionId;
}

function checkedHandle(value: unknown, expectedSlot?: string): ActionHandle {
  if (!isRecord(value) || !hasExactKeys(value, ["requestSlot", "generation"])
    || checkedRequestSlot(value.requestSlot) !== value.requestSlot
    || (expectedSlot !== undefined && value.requestSlot !== expectedSlot)
    || !Number.isSafeInteger(value.generation) || (value.generation as number) < 1) {
    throw new Error("action.invalid_handle");
  }
  return value as unknown as ActionHandle;
}

function checkedBoundedString(value: unknown, maxBytes: number, code: string): string {
  if (typeof value !== "string" || value.length === 0 || textEncoder.encode(value).byteLength > maxBytes) {
    throw new Error(code);
  }
  return value;
}

function checkedInput<K extends ActionId>(actionId: K, input: ActionInputById[K]): ActionInputById[K] {
  if (!isRecord(input)) throw new Error("action.invalid_input");
  const value: Record<string, unknown> = input;
  if (actionId === "capture.start") {
    if (!hasExactKeys(value, [])) throw new Error("action.invalid_input");
  } else if (actionId === "text.copy") {
    if (!hasExactKeys(value, ["text"])) throw new Error("action.invalid_input");
    checkedBoundedString(value.text, 256 * 1024, "action.invalid_input");
  } else if (actionId === "text.translate") {
    const keys = Object.keys(value);
    if (!keys.every(key => ["text", "sourceLanguage", "targetLanguage"].includes(key))
      || !Object.hasOwn(value, "text") || !Object.hasOwn(value, "targetLanguage")
      || keys.length < 2 || keys.length > 3) throw new Error("action.invalid_input");
    checkedBoundedString(value.text, 256 * 1024, "action.invalid_input");
    for (const [language, allowAuto] of [[value.targetLanguage, false], [value.sourceLanguage, true]] as const) {
      if (language === undefined) continue;
      if (typeof language !== "string" || language.length > 32
        || (!allowAuto && language === "auto") || !/^[A-Za-z0-9_-]+$/.test(language)) {
        throw new Error("action.invalid_input");
      }
    }
  } else {
    if (!hasExactKeys(value, ["sourceId", "sourceVersion"])
      || typeof value.sourceId !== "string" || textEncoder.encode(value.sourceId).byteLength > 128
      || !/^[A-Za-z0-9_-]+$/.test(value.sourceId)
      || !Number.isSafeInteger(value.sourceVersion) || (value.sourceVersion as number) < 0) {
      throw new Error("action.invalid_input");
    }
  }
  return input;
}

function checkedTranslation(value: unknown): TranslationResult {
  if (!isRecord(value) || !hasExactKeys(value, ["request_id", "provider", "translated_text", "detected_source_language", "target_language"])
    || !Number.isSafeInteger(value.request_id) || (value.request_id as number) < 0
    || !(["libretranslate", "openai_compatible", "deepl", "google", "bing", "youdao"] as TranslationProvider[]).includes(value.provider as TranslationProvider)
    || typeof value.translated_text !== "string" || textEncoder.encode(value.translated_text).byteLength > 1024 * 1024
    || !(value.detected_source_language === null
      || (typeof value.detected_source_language === "string" && value.detected_source_language.length <= 64))
    || typeof value.target_language !== "string" || value.target_language.length === 0 || value.target_language.length > 32) {
    throw new Error("action.invalid_translation");
  }
  return value as unknown as TranslationResult;
}

function checkedOutput<K extends ActionId>(actionId: K, value: unknown): ActionOutputById[K] {
  if (!isRecord(value)) throw new Error("action.invalid_output");
  if (actionId === "text.copy") {
    if (!hasExactKeys(value, ["type"]) || value.type !== "unit") throw new Error("action.invalid_output");
    return value as unknown as ActionOutputById[K];
  }
  if (!hasExactKeys(value, ["type", "value"])) throw new Error("action.invalid_output");
  switch (actionId) {
    case "capture.start":
      if (value.type !== "capture_session") throw new Error("action.invalid_output");
      checkedBoundedString(value.value, 128, "action.invalid_output");
      break;
    case "image.ocr":
      if (value.type !== "recognized_text") throw new Error("action.invalid_output");
      value.value = checkedStructuredOcr(value.value as StructuredOcr);
      break;
    case "image.pin":
      if (value.type !== "window_handle") throw new Error("action.invalid_output");
      checkedBoundedString(value.value, 128, "action.invalid_output");
      break;
    case "image.save":
      if (value.type !== "saved_path") throw new Error("action.invalid_output");
      checkedBoundedString(value.value, 16 * 1024, "action.invalid_output");
      break;
    case "image.scan_codes":
      if (value.type !== "detected_codes") throw new Error("action.invalid_output");
      value.value = parseImageCodeScanResponse(value.value);
      break;
    case "text.translate":
      if (value.type !== "translated_text") throw new Error("action.invalid_output");
      value.value = checkedTranslation(value.value);
      break;
  }
  return value as unknown as ActionOutputById[K];
}

export async function discoverActions(): Promise<ActionDescriptor[]> {
  const value = await invoke<unknown>("discover_actions");
  if (!Array.isArray(value) || value.length > Object.keys(CATALOG).length) {
    throw new Error("action.invalid_catalog");
  }
  const descriptors = value.map(checkedDescriptor);
  if (new Set(descriptors.map(descriptor => descriptor.id)).size !== descriptors.length) {
    throw new Error("action.invalid_catalog");
  }
  return descriptors;
}

export async function prepareAction<K extends ActionId>(
  actionId: K,
  requestSlot: string,
  input: ActionInputById[K],
): Promise<ActionHandle> {
  const id = checkedActionId(actionId);
  const slot = checkedRequestSlot(requestSlot);
  const handle = await invoke<unknown>("prepare_action", {
    actionId: id,
    requestSlot: slot,
    input: checkedInput(id, input),
  });
  return checkedHandle(handle, slot);
}

export async function runAction<K extends ActionId>(actionId: K, handle: ActionHandle): Promise<ActionReply<K>> {
  const id = checkedActionId(actionId);
  const checked = checkedHandle(handle);
  const value = await invoke<unknown>("run_action", { handle: checked });
  if (!isRecord(value) || !hasExactKeys(value, ["handle", "output"])) throw new Error("action.invalid_reply");
  const replyHandle = checkedHandle(value.handle, checked.requestSlot);
  if (replyHandle.generation !== checked.generation) throw new Error("action.stale_reply");
  return { handle: replyHandle, output: checkedOutput(id, value.output) } as ActionReply<K>;
}

export function cancelAction(handle: ActionHandle): Promise<void> {
  return invoke("cancel_action", { handle: checkedHandle(handle) });
}
