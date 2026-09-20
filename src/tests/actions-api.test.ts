import { beforeEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  cancelAction,
  actionLauncherReady,
  closeActionLauncher,
  discoverActions,
  getActionLauncherSettings,
  prepareAction,
  runAction,
  showActionLauncher,
  startActionLauncherDrag,
  type ActionDescriptor,
} from "../js/api.ts";

const captureDescriptor: ActionDescriptor = {
  id: "capture.start",
  input: "unit",
  output: "capture_session",
  permissions: ["screen.capture"],
  cancellable: false,
  platforms: ["linux", "windows", "macos"],
};

beforeEach(() => invoke.mockReset());

it("accepts only the exact static catalog subset without duplicates", async () => {
  invoke.mockResolvedValueOnce([captureDescriptor]);
  await expect(discoverActions()).resolves.toEqual([captureDescriptor]);
  expect(invoke).toHaveBeenCalledWith("discover_actions");

  invoke.mockResolvedValueOnce([{ ...captureDescriptor, permissions: ["file.write"] }]);
  await expect(discoverActions()).rejects.toThrow("invalid_descriptor");
  invoke.mockResolvedValueOnce([captureDescriptor, captureDescriptor]);
  await expect(discoverActions()).rejects.toThrow("invalid_catalog");
});

it("validates input once, returns a slot-bound handle, and runs with only that handle", async () => {
  const handle = { requestSlot: "capture.primary", generation: 3 };
  invoke.mockResolvedValueOnce(handle);
  await expect(prepareAction("capture.start", "capture.primary", {})).resolves.toEqual(handle);
  expect(invoke).toHaveBeenLastCalledWith("prepare_action", {
    actionId: "capture.start",
    requestSlot: "capture.primary",
    input: {},
  });

  invoke.mockResolvedValueOnce({
    handle,
    output: { type: "capture_session", value: "capture-session-3" },
  });
  await expect(runAction("capture.start", handle)).resolves.toEqual({
    handle,
    output: { type: "capture_session", value: "capture-session-3" },
  });
  expect(invoke).toHaveBeenLastCalledWith("run_action", { handle });
});

it("rejects forged handles, stale replies, mismatched outputs, and extra input fields", async () => {
  await expect(prepareAction("text.copy", "../copy", { text: "private" })).rejects.toThrow("invalid_request_slot");
  await expect(prepareAction("text.copy", "copy", { text: "private", extra: true } as never)).rejects.toThrow("invalid_input");
  expect(invoke).not.toHaveBeenCalled();

  const handle = { requestSlot: "copy", generation: 1 };
  invoke.mockResolvedValueOnce({ handle: { ...handle, generation: 2 }, output: { type: "unit" } });
  await expect(runAction("text.copy", handle)).rejects.toThrow("stale_reply");
  invoke.mockResolvedValueOnce({ handle, output: { type: "saved_path", value: "/private/file" } });
  await expect(runAction("text.copy", handle)).rejects.toThrow("invalid_output");
  await expect(runAction("text.copy", { requestSlot: "copy", generation: 0 })).rejects.toThrow("invalid_handle");
});

it("validates nested OCR and scan output before exposing it", async () => {
  const handle = { requestSlot: "analysis", generation: 1 };
  invoke.mockResolvedValueOnce({
    handle,
    output: {
      type: "recognized_text",
      value: {
        width: 1,
        height: 1,
        text: "A",
        lines: [],
        paragraphs: [],
        pipeline: { id: "fixture", engine: "tesseract", featureSchema: null, layoutExecuted: false, layoutReason: "unstructured_backend" },
        fallbackReason: "fixture",
      },
    },
  });
  await expect(runAction("image.ocr", handle)).resolves.toMatchObject({ output: { type: "recognized_text" } });

  invoke.mockResolvedValueOnce({
    handle,
    output: { type: "detected_codes", value: { results: [{ format: "aztec", text: "x", points: [] }], limited: false } },
  });
  await expect(runAction("image.scan_codes", handle)).rejects.toThrow("invalid image code scan format");
});

it("cancels with the exact validated handle", async () => {
  const handle = { requestSlot: "analysis", generation: 8 };
  invoke.mockResolvedValueOnce(undefined);
  await cancelAction(handle);
  expect(invoke).toHaveBeenCalledWith("cancel_action", { handle });
});

it("keeps launcher lifecycle on dedicated commands and validates its settings subset", async () => {
  invoke.mockResolvedValueOnce(undefined);
  await showActionLauncher();
  expect(invoke).toHaveBeenLastCalledWith("show_action_launcher");

  invoke.mockResolvedValueOnce({
    theme: "dark",
    language: "zh-CN",
    translationSourceLanguage: "auto",
    translationTargetLanguage: "en",
  });
  await expect(getActionLauncherSettings()).resolves.toMatchObject({ theme: "dark", language: "zh-CN" });
  invoke.mockResolvedValueOnce({
    theme: "dark",
    language: "zh-CN",
    translationSourceLanguage: "auto",
    translationTargetLanguage: "en",
    apiKey: "must-not-pass",
  });
  await expect(getActionLauncherSettings()).rejects.toThrow("invalid_settings");

  for (const [call, command] of [
    [actionLauncherReady, "action_launcher_ready"],
    [startActionLauncherDrag, "start_action_launcher_drag"],
    [closeActionLauncher, "close_action_launcher"],
  ] as const) {
    invoke.mockResolvedValueOnce(undefined);
    await call();
    expect(invoke).toHaveBeenLastCalledWith(command);
  }
});
