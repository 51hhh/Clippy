import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App, type RecordingLibraryServices } from "../react/recording-library/App.tsx";
import type { RecordingLibraryItem } from "../js/ipc-types.ts";

type Lease = Awaited<ReturnType<RecordingLibraryServices["preparePlayback"]>>;

const session: RecordingLibraryItem = {
  sessionId: "recording-1", state: "interrupted", createdAtUnixMs: 1_700_000_000_000,
  width: 640, height: 360, targetFpsNumerator: 30, targetFpsDenominator: 1,
  encoder: "vp9-prototype", container: "webm", includeCursor: true, droppedFrames: 0,
  durationMs: 2000, frameCount: 60, byteLength: 1024, canMerge: false, canThumbnail: false,
  artifacts: [0, 1].map((index) => ({
    artifactId: `segment-${String(index).padStart(6, "0")}`,
    displayName: `segment-${index}.webm`, durationMs: 1000, frameCount: 30, byteLength: 512,
  })),
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(): RecordingLibraryServices {
  return {
    ready: vi.fn(async () => {}), list: vi.fn(async () => [session]),
    thumbnail: vi.fn(async () => null), exportArtifact: vi.fn(async () => true),
    revealArtifact: vi.fn(async () => {}), deleteSession: vi.fn(async () => {}),
    mergeSession: vi.fn(async () => {}), preparePlayback: vi.fn(async () => lease(1)),
    releasePlayback: vi.fn(async () => {}),
    mediaUrl: vi.fn((token: string) => `recording-media://localhost/${token}`),
    startDrag: vi.fn(async () => {}), close: vi.fn(async () => {}),
  };
}

function lease(index: number): Lease {
  return { token: `media-${String(index).padStart(16, "0")}`, mimeType: "video/webm" };
}

function button(label: string): HTMLButtonElement {
  return [...document.querySelectorAll<HTMLButtonElement>("button")]
    .find((candidate) => candidate.textContent === label)!;
}

describe("recording playback lifecycle", () => {
  let root: Root;
  let services: RecordingLibraryServices;
  const reactEnvironment = globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean };

  beforeEach(() => {
    reactEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    root = createRoot(document.getElementById("root")!);
    services = fixture();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete reactEnvironment.IS_REACT_ACT_ENVIRONMENT;
  });

  async function render(owner = services) {
    await act(async () => root.render(<App services={owner} />));
  }

  async function click(label: string) {
    const target = button(label);
    expect(target).toBeDefined();
    expect(target.disabled).toBe(false);
    await act(async () => target.click());
  }

  it("releases a late prepared lease after the component unmounts", async () => {
    const pending = deferred<Lease>();
    vi.mocked(services.preparePlayback).mockReturnValueOnce(pending.promise);
    await render();
    await click("Play");
    await act(async () => root.render(<></>));
    await act(async () => pending.resolve(lease(2)));
    expect(services.releasePlayback).toHaveBeenCalledExactlyOnceWith(lease(2).token);
    expect(services.mediaUrl).not.toHaveBeenCalled();
    expect(document.querySelector("video")).toBeNull();
  });

  it("handles a rejected late release after unmount", async () => {
    const pending = deferred<Lease>();
    vi.mocked(services.preparePlayback).mockReturnValueOnce(pending.promise);
    vi.mocked(services.releasePlayback).mockRejectedValueOnce(new Error("already closed"));
    await render();
    await click("Play");
    await act(async () => root.render(<></>));
    await act(async () => pending.resolve(lease(2)));
    expect(services.releasePlayback).toHaveBeenCalledExactlyOnceWith(lease(2).token);
    expect(services.mediaUrl).not.toHaveBeenCalled();
  });

  it("releases an old services response through its original owner", async () => {
    const pending = deferred<Lease>();
    vi.mocked(services.preparePlayback).mockReturnValueOnce(pending.promise);
    await render();
    await click("Play");
    const replacement = fixture();
    await render(replacement);
    await act(async () => pending.resolve(lease(2)));
    expect(services.releasePlayback).toHaveBeenCalledExactlyOnceWith(lease(2).token);
    expect(replacement.releasePlayback).not.toHaveBeenCalled();
    expect(services.mediaUrl).not.toHaveBeenCalled();
    expect(document.querySelector("video")).toBeNull();
    await click("Play");
    expect(replacement.preparePlayback).toHaveBeenCalledOnce();
    expect(document.querySelector("video source")?.getAttribute("src")).toContain(lease(1).token);
  });

  it("closing the current preview cancels a pending switch", async () => {
    const pending = deferred<Lease>();
    vi.mocked(services.preparePlayback).mockResolvedValueOnce(lease(1)).mockReturnValueOnce(pending.promise);
    await render();
    await click("Play");
    await click("Play");
    await click("Close preview");
    expect(document.querySelector("video")).toBeNull();
    await act(async () => pending.resolve(lease(2)));
    expect(document.querySelector("video")).toBeNull();
    expect(services.releasePlayback).toHaveBeenCalledTimes(2);
    expect(services.releasePlayback).toHaveBeenNthCalledWith(1, lease(1).token);
    expect(services.releasePlayback).toHaveBeenNthCalledWith(2, lease(2).token);
    expect(services.mediaUrl).toHaveBeenCalledOnce();
    expect(button("Play").disabled).toBe(false);
  });

  it("an old rejection cannot clear a replacement request busy state or show an error", async () => {
    const old = deferred<Lease>();
    const current = deferred<Lease>();
    vi.mocked(services.preparePlayback).mockReturnValueOnce(old.promise);
    await render();
    await click("Play");
    const replacement = fixture();
    vi.mocked(replacement.preparePlayback).mockReturnValueOnce(current.promise);
    await render(replacement);
    await click("Play");
    await act(async () => old.reject(new Error("old preparation failed")));
    expect(document.querySelector('[role="status"]')).toBeNull();
    expect(button("Export").disabled).toBe(true);
    await act(async () => current.resolve(lease(3)));
    expect(document.querySelector("video source")?.getAttribute("src")).toContain(lease(3).token);
    expect(button("Export").disabled).toBe(false);
    expect(services.releasePlayback).not.toHaveBeenCalled();
  });

  it("a current preparation failure remains visible and retryable", async () => {
    vi.mocked(services.preparePlayback).mockRejectedValueOnce(new Error("damaged media"));
    await render();
    await click("Play");
    expect(document.querySelector('[role="status"]')?.textContent).toContain("action failed");
    expect(button("Play").disabled).toBe(false);
    await click("Play");
    expect(document.querySelector("video source")?.getAttribute("src")).toContain(lease(1).token);
  });

  it("a media URL failure releases the prepared lease exactly once", async () => {
    vi.mocked(services.mediaUrl).mockImplementationOnce(() => { throw new Error("invalid URL"); });
    await render();
    await click("Play");
    expect(services.releasePlayback).toHaveBeenCalledExactlyOnceWith(lease(1).token);
    expect(document.querySelector('[role="status"]')?.textContent).toContain("action failed");
    expect(document.querySelector("video")).toBeNull();
    expect(button("Play").disabled).toBe(false);
  });

  it("closing a preview preserves an unrelated export busy state", async () => {
    const exporting = deferred<boolean>();
    vi.mocked(services.exportArtifact).mockReturnValueOnce(exporting.promise);
    await render();
    await click("Play");
    await click("Export");
    await click("Close preview");
    expect(document.querySelector("video")).toBeNull();
    expect(services.releasePlayback).toHaveBeenCalledExactlyOnceWith(lease(1).token);
    expect(button("Play").disabled).toBe(true);
    await act(async () => exporting.resolve(true));
    expect(button("Play").disabled).toBe(false);
  });
});
