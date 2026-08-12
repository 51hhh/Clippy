import { describe, expect, it, vi } from "vitest";
import { createPanelVisibilityController } from "../js/panel-visibility.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

describe("panel visibility transaction", () => {
  it("serializes persistence before committing the latest request", async () => {
    const first = deferred();
    const persist = vi.fn()
      .mockImplementationOnce(() => first.promise)
      .mockResolvedValueOnce(undefined);
    const apply = vi.fn();
    const controller = createPanelVisibilityController({ apply, persist });

    const opening = controller.request(true);
    const closing = controller.request(false);
    await Promise.resolve();
    expect(persist).toHaveBeenCalledTimes(1);
    first.resolve();
    await opening;
    await closing;

    expect(persist.mock.calls).toEqual([[true], [false]]);
    expect(controller.isVisible()).toBe(false);
  });

  it("restores the last committed state after the latest request fails", async () => {
    const persist = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("resize"));
    const apply = vi.fn();
    const controller = createPanelVisibilityController({ apply, persist });

    await controller.request(true);
    await expect(controller.request(false)).rejects.toThrow("resize");

    expect(controller.isVisible()).toBe(true);
    expect(apply).toHaveBeenLastCalledWith(true);
  });
});
