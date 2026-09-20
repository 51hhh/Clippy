import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  App,
  formatRecordingBytes,
  formatRecordingDuration,
  type RecordingLibraryServices,
} from "../react/recording-library/App.tsx";
import type { RecordingLibraryItem } from "../js/ipc-types.ts";

const complete: RecordingLibraryItem = {
  sessionId: "recording-1",
  state: "complete",
  createdAtUnixMs: 1_700_000_000_000,
  width: 1920,
  height: 1080,
  targetFpsNumerator: 30,
  targetFpsDenominator: 1,
  encoder: "vp9",
  container: "webm",
  includeCursor: true,
  droppedFrames: 0,
  durationMs: 65_000,
  frameCount: 1950,
  byteLength: 2048,
  artifacts: [{
    artifactId: "final",
    displayName: "recording.webm",
    durationMs: 65_000,
    frameCount: 1950,
    byteLength: 2048,
  }],
};

describe("recording library app", () => {
  let root: Root;
  let services: RecordingLibraryServices;
  const reactEnvironment = globalThis as typeof globalThis & {
    IS_REACT_ACT_ENVIRONMENT?: boolean;
  };

  beforeEach(() => {
    reactEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    services = {
      ready: vi.fn(async () => {}),
      list: vi.fn(async () => [complete]),
      exportArtifact: vi.fn(async () => true),
      revealArtifact: vi.fn(async () => {}),
      deleteSession: vi.fn(async () => {}),
      startDrag: vi.fn(async () => {}),
      close: vi.fn(async () => {}),
    };
    root = createRoot(document.getElementById("root")!);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete reactEnvironment.IS_REACT_ACT_ENVIRONMENT;
  });

  async function render() {
    await act(async () => root.render(<App services={services} />));
    await act(async () => Promise.resolve());
  }

  it("formats duration and byte sizes", () => {
    expect(formatRecordingDuration(65_000)).toBe("1:05");
    expect(formatRecordingDuration(3_661_000)).toBe("1:01:01");
    expect(formatRecordingBytes(512)).toBe("512 B");
    expect(formatRecordingBytes(2048)).toBe("2.0 KB");
  });

  it("shows complete artifacts and routes export and reveal through opaque ids", async () => {
    await render();
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("recording.webm");

    const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
    await act(async () => buttons.find((button) => button.textContent === "Export")!.click());
    expect(services.exportArtifact).toHaveBeenCalledWith("recording-1", "final");
    await act(async () => buttons.find((button) => button.textContent === "Show in folder")!.click());
    expect(services.revealArtifact).toHaveBeenCalledWith("recording-1", "final");
  });

  it("uses an inline confirmation before deleting and reloads after success", async () => {
    await render();
    const deleteButton = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Delete")!;
    await act(async () => deleteButton.click());
    expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
    expect(services.deleteSession).not.toHaveBeenCalled();

    const confirm = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((button) => button.textContent === "Delete").at(-1)!;
    await act(async () => confirm.click());
    expect(services.deleteSession).toHaveBeenCalledWith("recording-1");
    expect(services.list).toHaveBeenCalledTimes(2);
  });

  it("shows an empty state and offers retry after a loading error", async () => {
    vi.mocked(services.list).mockRejectedValueOnce(new Error("failed")).mockResolvedValueOnce([]);
    await render();
    expect(document.querySelector('[role="status"]')?.textContent).toContain("could not be loaded");
    const retry = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Retry")!;
    await act(async () => retry.click());
    expect(document.body.textContent).toContain("No recordings yet");
  });
});
