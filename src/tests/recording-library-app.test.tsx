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
  canMerge: false,
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
      mergeSession: vi.fn(async () => {}),
      preparePlayback: vi.fn(async () => ({
        token: "media-0000000000000001",
        mimeType: "video/webm" as const,
      })),
      releasePlayback: vi.fn(async () => {}),
      mediaUrl: vi.fn((token: string) => `recording-media://localhost/${token}`),
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

  it("prepares one opaque WebM lease and releases it when the preview closes", async () => {
    await render();
    const play = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Play")!;
    await act(async () => play.click());
    expect(services.preparePlayback).toHaveBeenCalledWith("recording-1", "final");
    expect(services.mediaUrl).toHaveBeenCalledWith("media-0000000000000001");
    expect(document.querySelector("video source")?.getAttribute("src"))
      .toBe("recording-media://localhost/media-0000000000000001");

    const close = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Close preview")!;
    await act(async () => close.click());
    expect(services.releasePlayback).toHaveBeenCalledWith("media-0000000000000001");
    expect(document.querySelector("video")).toBeNull();
  });

  it("releases the current playback before deleting its session", async () => {
    await render();
    const play = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Play")!;
    await act(async () => play.click());

    const deleteButton = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Delete")!;
    await act(async () => deleteButton.click());
    const confirm = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((button) => button.textContent === "Delete").at(-1)!;
    await act(async () => confirm.click());

    expect(services.releasePlayback).toHaveBeenCalledWith("media-0000000000000001");
    expect(services.deleteSession).toHaveBeenCalledWith("recording-1");
  });

  it("keeps AVI diagnostic artifacts export-only", async () => {
    vi.mocked(services.list).mockResolvedValueOnce([{
      ...complete,
      state: "interrupted",
      container: "avi",
      canMerge: false,
    }]);
    await render();
    expect([...document.querySelectorAll("button")].some((button) => button.textContent === "Play")).toBe(false);
    expect([...document.querySelectorAll("button")].some((button) => button.textContent === "Recover recording")).toBe(false);
    expect(document.body.textContent).toContain("Export");
  });

  it("shows a recoverable inline error when WebView decoding fails", async () => {
    await render();
    const play = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Play")!;
    await act(async () => play.click());
    await act(async () => document.querySelector("video")!.dispatchEvent(new Event("error")));
    expect(document.querySelector('[role="status"]')?.textContent).toContain("action failed");
    expect(document.body.textContent).toContain("Export");
  });

  it("releases the previous lease when switching recoverable segments", async () => {
    vi.mocked(services.list).mockResolvedValueOnce([{
      ...complete,
      state: "interrupted",
      artifacts: [
        { ...complete.artifacts[0], artifactId: "segment-000000", displayName: "segment-000000.webm" },
        { ...complete.artifacts[0], artifactId: "segment-000001", displayName: "segment-000001.webm" },
      ],
    }]);
    vi.mocked(services.preparePlayback)
      .mockResolvedValueOnce({ token: "media-0000000000000001", mimeType: "video/webm" })
      .mockResolvedValueOnce({ token: "media-0000000000000002", mimeType: "video/webm" });
    await render();
    let play = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((button) => button.textContent === "Play");
    await act(async () => play[0].click());
    play = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((button) => button.textContent === "Play");
    await act(async () => play[0].click());
    expect(services.preparePlayback).toHaveBeenLastCalledWith("recording-1", "segment-000001");
    expect(services.releasePlayback).toHaveBeenCalledWith("media-0000000000000001");
    expect(document.querySelector("video source")?.getAttribute("src"))
      .toContain("media-0000000000000002");
  });

  it("recovers an interrupted VP9 session and reloads it as one recording", async () => {
    const interrupted: RecordingLibraryItem = {
      ...complete,
      state: "interrupted",
      canMerge: true,
      artifacts: [
        { ...complete.artifacts[0], artifactId: "segment-000000", displayName: "segment-000000.webm" },
        { ...complete.artifacts[0], artifactId: "segment-000001", displayName: "segment-000001.webm" },
      ],
    };
    vi.mocked(services.list)
      .mockResolvedValueOnce([interrupted])
      .mockResolvedValueOnce([complete]);
    await render();

    const recover = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Recover recording")!;
    await act(async () => recover.click());

    expect(services.mergeSession).toHaveBeenCalledWith("recording-1");
    expect(services.list).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("recording.webm");
    expect(document.body.textContent).not.toContain("segment-000000.webm");
  });

  it("keeps recoverable segments and offers retry when remux fails", async () => {
    vi.mocked(services.list).mockResolvedValueOnce([{
      ...complete,
      state: "interrupted",
      canMerge: true,
      artifacts: [{
        ...complete.artifacts[0],
        artifactId: "segment-000000",
        displayName: "segment-000000.webm",
      }],
    }]);
    vi.mocked(services.mergeSession).mockRejectedValueOnce(new Error("invalid segment"));
    await render();

    const recover = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Recover recording")!;
    await act(async () => recover.click());

    expect(document.querySelector('[role="status"]')?.textContent).toContain("action failed");
    expect(document.body.textContent).toContain("segment-000000.webm");
    expect(document.body.textContent).toContain("Recover recording");
    expect(services.list).toHaveBeenCalledOnce();
  });

  it("lets an in-flight recovery finish after the results window closes", async () => {
    const interrupted: RecordingLibraryItem = {
      ...complete,
      state: "interrupted",
      canMerge: true,
      artifacts: [{
        ...complete.artifacts[0],
        artifactId: "segment-000000",
        displayName: "segment-000000.webm",
      }],
    };
    let finishMerge: (() => void) | undefined;
    vi.mocked(services.list).mockResolvedValueOnce([interrupted]);
    vi.mocked(services.mergeSession).mockImplementationOnce(() => new Promise<void>((resolve) => {
      finishMerge = resolve;
    }));
    await render();

    const recover = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Recover recording")!;
    await act(async () => recover.click());
    await act(async () => root.render(<></>));
    finishMerge!();
    await Promise.resolve();

    expect(services.mergeSession).toHaveBeenCalledWith("recording-1");
    expect(services.list).toHaveBeenCalledOnce();
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
