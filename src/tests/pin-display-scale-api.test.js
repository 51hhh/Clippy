import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  current: { onScaleChanged: vi.fn() },
  getCurrentWindow: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, convertFileSrc: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: mocks.getCurrentWindow }));

import { getPinDisplayScale, onPinDisplayScaleChanged } from "../js/api.ts";
import { pinApi } from "../react/pin/api.ts";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getCurrentWindow.mockReturnValue(mocks.current);
});

it("reads the caller-bound command without a target label or a broad native getter", async () => {
  mocks.invoke.mockResolvedValueOnce(1.25).mockResolvedValueOnce(1.5);
  expect(await getPinDisplayScale()).toBe(1.25);
  expect(await pinApi.displayScale()).toBe(1.5);
  expect(mocks.invoke.mock.calls).toEqual([["get_pin_display_scale"], ["get_pin_display_scale"]]);
  expect(mocks.getCurrentWindow).not.toHaveBeenCalled();
});

it("routes current-window scale events through the facade and returns the native disposer", async () => {
  const dispose = vi.fn();
  const notify = vi.fn();
  mocks.current.onScaleChanged.mockResolvedValue(dispose);
  expect(await pinApi.onDisplayScaleChanged(notify)).toBe(dispose);
  expect(mocks.getCurrentWindow).toHaveBeenCalledTimes(1);
  const callback = mocks.current.onScaleChanged.mock.calls[0][0];
  callback({ payload: { scaleFactor: 1.25, size: { width: 1000, height: 750 } } });
  expect(notify).toHaveBeenCalledWith(1.25);
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("propagates a failed native subscription without inventing a disposer", async () => {
  mocks.current.onScaleChanged.mockRejectedValueOnce(new Error("subscription denied"));
  await expect(onPinDisplayScaleChanged(vi.fn())).rejects.toThrow("subscription denied");
});
