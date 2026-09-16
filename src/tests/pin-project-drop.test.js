import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../i18n/i18n.js", () => ({
  t: (key) => key,
}));

import { __test__, initPinProjectDrop } from "../js/pin-project-drop.js";

afterEach(() => {
  document.body.replaceChildren();
});

describe("editable PNG project drop entry", () => {
  it("only accepts one PNG path before handing it to the backend boundary", () => {
    expect(__test__.isSinglePng(["/tmp/project.PNG"])).toBe(true);
    expect(__test__.isSinglePng([])).toBe(false);
    expect(__test__.isSinglePng(["/tmp/a.png", "/tmp/b.png"])).toBe(false);
    expect(__test__.isSinglePng(["/tmp/photo.jpg"])).toBe(false);
  });

  it("shows an accessible prompt only during a drag and forwards one PNG drop", async () => {
    let listener;
    const unlisten = vi.fn();
    const subscribe = vi.fn((callback) => {
      listener = callback;
      return Promise.resolve(unlisten);
    });
    const openProject = vi.fn(() => Promise.resolve("pin-image-1"));

    const dispose = await initPinProjectDrop({ subscribe, openProject });
    const overlay = document.querySelector(".pin-project-drop-overlay");
    expect(overlay.hidden).toBe(true);
    expect(overlay.getAttribute("role")).toBe("status");
    expect(overlay.getAttribute("aria-live")).toBe("polite");

    listener({ type: "enter", paths: ["/tmp/project.png"], position: { x: 0, y: 0 } });
    expect(overlay.hidden).toBe(false);
    listener({ type: "leave" });
    expect(overlay.hidden).toBe(true);

    listener({ type: "drop", paths: ["/tmp/project.png"], position: { x: 0, y: 0 } });
    await Promise.resolve();
    expect(openProject).toHaveBeenCalledWith("/tmp/project.png");
    expect(overlay.hidden).toBe(true);

    listener({ type: "drop", paths: ["/tmp/plain.png", "/tmp/second.png"], position: { x: 0, y: 0 } });
    expect(openProject).toHaveBeenCalledOnce();

    dispose();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(document.querySelector(".pin-project-drop-overlay")).toBeNull();
  });

  it("reports a backend-rejected ordinary PNG instead of treating it as opened", async () => {
    let listener;
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    const subscribe = vi.fn((callback) => {
      listener = callback;
      return Promise.resolve(vi.fn());
    });
    const openProject = vi.fn(() => Promise.resolve(null));

    const dispose = await initPinProjectDrop({ subscribe, openProject });
    listener({ type: "drop", paths: ["/tmp/plain.png"], position: { x: 0, y: 0 } });
    await Promise.resolve();

    expect(openProject).toHaveBeenCalledWith("/tmp/plain.png");
    expect(info).toHaveBeenCalledWith("pinProject.dropRejected");
    const overlay = document.querySelector(".pin-project-drop-overlay");
    expect(overlay.hidden).toBe(false);
    expect(overlay.textContent).toBe("pinProject.dropRejected");
    info.mockRestore();
    dispose();
  });

  it("removes the overlay when native drag subscription fails", async () => {
    const failure = new Error("native drag unavailable");

    await expect(initPinProjectDrop({
      subscribe: vi.fn(() => Promise.reject(failure)),
    })).rejects.toBe(failure);

    expect(document.querySelector(".pin-project-drop-overlay")).toBeNull();
  });
});
