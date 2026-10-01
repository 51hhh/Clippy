import React, { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App, type RecordingLibraryServices } from "../react/recording-library/App.tsx";
import type { RecordingLibraryItem } from "../js/ipc-types.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(): RecordingLibraryServices {
  return {
    ready: vi.fn(async () => {}), list: vi.fn(async () => []),
    thumbnail: vi.fn(async () => null), exportArtifact: vi.fn(async () => true),
    revealArtifact: vi.fn(async () => {}), deleteSession: vi.fn(async () => {}),
    mergeSession: vi.fn(async () => {}),
    preparePlayback: vi.fn(async () => ({ token: "media-0000000000000001", mimeType: "video/webm" as const })),
    releasePlayback: vi.fn(async () => {}), mediaUrl: vi.fn((token: string) => token),
    startDrag: vi.fn(async () => {}), close: vi.fn(async () => {}),
  };
}

describe("recording library ready lifecycle", () => {
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

  it("the active load requests ready once only after completion", async () => {
    const pending = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(pending.promise);
    await render();
    expect(services.ready).not.toHaveBeenCalled();
    await act(async () => pending.resolve([]));
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("No recordings yet");
  });

  it("the active failed load still requests ready for the retry page", async () => {
    vi.mocked(services.list).mockRejectedValueOnce(new Error("storage unavailable"));
    await render();
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="status"]')?.textContent).toContain("could not be loaded");
    expect(document.body.textContent).toContain("Retry");
  });

  it("an active ready rejection remains handled without hiding loaded content", async () => {
    vi.mocked(services.ready).mockRejectedValueOnce(new Error("window already closed"));
    await render();
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("No recordings yet");
    expect(document.querySelector('[role="status"]')).toBeNull();
  });

  it("a successful load completing after unmount does not request ready", async () => {
    const pending = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(pending.promise);
    await render();
    await act(async () => root.render(<></>));
    await act(async () => pending.resolve([]));
    expect(services.ready).not.toHaveBeenCalled();
    expect(document.querySelector("main")).toBeNull();
  });

  it("a failed load completing after unmount does not request ready", async () => {
    const pending = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(pending.promise);
    await render();
    await act(async () => root.render(<></>));
    await act(async () => pending.reject(new Error("late failure")));
    expect(services.ready).not.toHaveBeenCalled();
    expect(document.querySelector("main")).toBeNull();
  });

  it("a replaced services success cannot request ready through its old owner", async () => {
    const pending = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(pending.promise);
    await render();
    const replacement = fixture();
    await render(replacement);
    expect(replacement.ready).toHaveBeenCalledOnce();
    await act(async () => pending.resolve([]));
    expect(services.ready).not.toHaveBeenCalled();
    expect(replacement.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("No recordings yet");
  });

  it("StrictMode retired load cannot repeat the current ready request", async () => {
    const retired = deferred<RecordingLibraryItem[]>();
    const current = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(retired.promise).mockReturnValueOnce(current.promise);
    await act(async () => root.render(<StrictMode><App services={services} /></StrictMode>));
    expect(services.list).toHaveBeenCalledTimes(2);
    expect(services.ready).not.toHaveBeenCalled();
    await act(async () => current.resolve([]));
    expect(services.ready).toHaveBeenCalledOnce();
    await act(async () => retired.resolve([]));
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("No recordings yet");
  });

  it("a replaced services failure cannot request ready or corrupt the current page", async () => {
    const pending = deferred<RecordingLibraryItem[]>();
    vi.mocked(services.list).mockReturnValueOnce(pending.promise);
    await render();
    const replacement = fixture();
    await render(replacement);
    await act(async () => pending.reject(new Error("retired storage failure")));
    expect(services.ready).not.toHaveBeenCalled();
    expect(replacement.ready).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="status"]')).toBeNull();
    expect(document.body.textContent).toContain("No recordings yet");
  });
});
