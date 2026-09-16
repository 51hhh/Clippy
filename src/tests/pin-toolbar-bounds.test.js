import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { usePinToolbarBounds } from "../react/pin/usePinToolbarBounds";

let root;
afterEach(async () => {
  if (root) await act(async () => root.unmount());
  vi.useRealTimers();
  delete globalThis.IS_REACT_ACT_ENVIRONMENT;
});

it("accepts native toolbar bounds after StrictMode replays the mount effects", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  const loadBounds = vi.fn(async () => ({ x: 0, y: 0, width: 240, height: 220 }));
  function Probe() {
    const bounds = usePinToolbarBounds("pin-review", { width: 800, height: 600 }, loadBounds);
    return React.createElement("output", null, `${bounds.width} × ${bounds.height}`);
  }
  const host = document.createElement("div"); document.body.replaceChildren(host);
  root = createRoot(host);
  await act(async () => root.render(React.createElement(StrictMode, null, React.createElement(Probe))));
  await act(async () => vi.advanceTimersByTimeAsync(180));
  expect(loadBounds).toHaveBeenCalledWith("pin-review");
  expect(host.textContent).toBe("240 × 220");
});
