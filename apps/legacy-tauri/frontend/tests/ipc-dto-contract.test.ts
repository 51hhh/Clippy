// @vitest-environment node
import { describe, expect, it } from "vitest";
import type {
  CaptureSelection,
  PinCanvasSaveResult,
  ViewerHandle,
} from "../js/ipc-types.ts";
import captureSelectionFixture from "./fixtures/ipc-contract/capture-selection.json";
import pinCanvasSaveResultFixture from "./fixtures/ipc-contract/pin-canvas-save-result.json";
import viewerHandleFixture from "./fixtures/ipc-contract/viewer-handle.json";

const captureSelection: CaptureSelection = captureSelectionFixture;
const pinCanvasSaveResult: PinCanvasSaveResult = pinCanvasSaveResultFixture;
const viewerHandle: ViewerHandle = viewerHandleFixture;

describe("shared IPC DTO fixtures", () => {
  it("keeps the capture selection wire keys stable", () => {
    expect(captureSelection).toEqual(captureSelectionFixture);
    expect(Object.keys(captureSelection).sort()).toEqual([
      "height", "monitorId", "sessionId", "width", "x", "y",
    ]);
  });

  it("keeps the pin save result omission rule stable", () => {
    expect(pinCanvasSaveResult).toEqual(pinCanvasSaveResultFixture);
    expect(Object.keys(pinCanvasSaveResult).sort()).toEqual(["clipboardWritten", "path"]);
  });

  it("keeps the viewer handle identity keys stable", () => {
    expect(viewerHandle).toEqual(viewerHandleFixture);
    expect(Object.keys(viewerHandle).sort()).toEqual(["sessionId", "snapshotId"]);
  });
});
