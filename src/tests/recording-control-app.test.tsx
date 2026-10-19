import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  App,
  formatElapsed,
  type RecordingControlServices,
} from "../react/recording-control/App.tsx";

describe("recording control app", () => {
  let root: Root;
  let services: RecordingControlServices;
  const reactEnvironment = globalThis as typeof globalThis & {
    IS_REACT_ACT_ENVIRONMENT?: boolean;
  };

  beforeEach(async () => {
    reactEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    services = {
      ready: vi.fn(async () => {}),
      pause: vi.fn(async () => {}),
      health: vi.fn(async () => {}),
      resume: vi.fn(async () => {}),
      stop: vi.fn(async () => ({
        outputAvailable: true,
        durationMs: 1000,
        capturedFrames: 10,
        encodedFrames: 10,
        droppedByBackpressure: 0,
      })),
      cancel: vi.fn(async () => {}),
    };
    root = createRoot(document.getElementById("root")!);
    await act(async () => root.render(<App services={services} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    vi.useRealTimers();
    delete reactEnvironment.IS_REACT_ACT_ENVIRONMENT;
  });

  it("formats long durations without wrapping the hour counter", () => {
    expect(formatElapsed(0)).toBe("00:00");
    expect(formatElapsed(65_999)).toBe("01:05");
    expect(formatElapsed(3_661_000)).toBe("01:01:01");
  });

  it("reports readiness and serializes pause resume and stop controls", async () => {
    expect(services.ready).toHaveBeenCalledOnce();
    const pause = document.querySelector<HTMLButtonElement>('[aria-label="Pause recording"]')!;
    await act(async () => pause.click());
    expect(services.pause).toHaveBeenCalledOnce();

    const resume = document.querySelector<HTMLButtonElement>('[aria-label="Resume recording"]')!;
    await act(async () => resume.click());
    expect(services.resume).toHaveBeenCalledOnce();

    const stop = document.querySelector<HTMLButtonElement>('[aria-label="Stop recording"]')!;
    await act(async () => stop.click());
    expect(services.stop).toHaveBeenCalledOnce();
    expect(stop.disabled).toBe(true);
  });

  it("freezes controls when a backend command fails", async () => {
    vi.mocked(services.pause).mockRejectedValueOnce(new Error("failed"));
    const pause = document.querySelector<HTMLButtonElement>('[aria-label="Pause recording"]')!;
    await act(async () => pause.click());
    expect(document.querySelector('[role="status"]')?.textContent).toBe("Control failed");
    expect(document.querySelectorAll<HTMLButtonElement>("button:disabled")).toHaveLength(2);
  });

  it("polls backend health and freezes controls after an asynchronous worker failure", async () => {
    await act(async () => root.unmount());
    vi.useFakeTimers();
    vi.mocked(services.health).mockRejectedValueOnce(new Error("worker failed"));
    root = createRoot(document.getElementById("root")!);
    await act(async () => root.render(<App services={services} />));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(services.health).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="status"]')?.textContent).toBe("Control failed");
    expect(document.querySelectorAll<HTMLButtonElement>("button:disabled")).toHaveLength(2);
  });

  it("shows a cancellable system authorization state without recording controls", async () => {
    await act(async () => root.unmount());
    root = createRoot(document.getElementById("root")!);
    await act(async () => root.render(<App services={services} mode="authorization" />));

    expect(document.querySelector('[aria-label="Recording authorization"]')).not.toBeNull();
    expect(document.querySelector('[aria-label="Pause recording"]')).toBeNull();
    const cancel = Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "Cancel",
    )!;
    await act(async () => cancel.click());
    expect(services.cancel).toHaveBeenCalledOnce();
    expect(cancel.textContent).toBe("Cancelling…");
  });
});
