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
      resume: vi.fn(async () => {}),
      stop: vi.fn(async () => ({
        outputAvailable: true,
        durationMs: 1000,
        capturedFrames: 10,
        encodedFrames: 10,
        droppedByBackpressure: 0,
      })),
    };
    root = createRoot(document.getElementById("root")!);
    await act(async () => root.render(<App services={services} />));
  });

  afterEach(async () => {
    await act(async () => root.unmount());
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
});
