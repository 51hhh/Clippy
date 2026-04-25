import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as telemetry from "../js/telemetry.js";

vi.mock("../js/api.js", () => ({
  getClips: vi.fn(),
  deleteClip: vi.fn(),
  toggleFavorite: vi.fn(),
  selectClip: vi.fn(),
}));

import * as api from "../js/api.js";
import * as clipboardList from "../js/clipboard-list.js";

function makeClip(overrides = {}) {
  return {
    id: 1,
    text_content: "hello world",
    is_favorite: false,
    byte_size: 11,
    created_at: Math.floor(Date.now() / 1000),
    ...overrides,
  };
}

describe("clipboard-list 渲染", () => {
  let parent;
  let emptyState;
  let events;

  beforeEach(() => {
    document.body.innerHTML = `
      <div id="clip-list"></div>
      <div id="empty-state" class="hidden"></div>
    `;
    parent = document.getElementById("clip-list");
    emptyState = document.getElementById("empty-state");
    clipboardList.init(parent, emptyState);

    events = [];
    telemetry.enable({ bufferLimit: 50 });
    telemetry.subscribe((rec) => events.push(rec));

    api.getClips.mockReset();
    api.toggleFavorite.mockReset();
    api.selectClip.mockReset();
    api.deleteClip.mockReset();
  });

  afterEach(() => {
    telemetry.disable();
  });

  it("正常列表渲染为安全 textContent，并发出 telemetry", async () => {
    const malicious = '<img src=x onerror="alert(1)">';
    api.getClips.mockResolvedValueOnce([
      makeClip({ id: 1, text_content: malicious }),
      makeClip({ id: 2, text_content: "normal", is_favorite: true }),
    ]);

    await clipboardList.refresh();

    const items = parent.querySelectorAll(".clip-item");
    expect(items.length).toBe(2);

    const previewEl = items[0].querySelector(".clip-preview");
    expect(previewEl.textContent).toBe(malicious);
    expect(previewEl.querySelector("img")).toBeNull();
    expect(parent.querySelectorAll("img").length).toBe(0);

    expect(items[1].classList.contains("favorite")).toBe(true);
    expect(items[1].querySelector(".clip-star").textContent).toBe("★");

    const refreshEvt = events.find((e) => e.event === "clip-list:refresh");
    expect(refreshEvt).toBeTruthy();
    expect(refreshEvt.payload.count).toBe(2);

    const renderEvt = events.find((e) => e.event === "clip-list:render");
    expect(renderEvt.payload).toEqual({ count: 2, empty: false });
  });

  it("空列表显示 empty-state 并发出 empty=true", async () => {
    api.getClips.mockResolvedValueOnce([]);
    await clipboardList.refresh();

    expect(parent.classList.contains("hidden")).toBe(true);
    expect(emptyState.classList.contains("hidden")).toBe(false);

    const renderEvt = events.find((e) => e.event === "clip-list:render");
    expect(renderEvt.payload).toEqual({ count: 0, empty: true });
  });

  it("查询失败时清空列表并报错事件", async () => {
    api.getClips.mockRejectedValueOnce(new Error("boom"));
    await clipboardList.refresh();

    expect(parent.querySelectorAll(".clip-item").length).toBe(0);
    const errEvt = events.find((e) => e.event === "clip-list:refresh-error");
    expect(errEvt).toBeTruthy();
    expect(errEvt.payload.message).toContain("boom");
  });

  it("点击星标走 toggleFavorite + 重新拉取", async () => {
    api.getClips.mockResolvedValue([makeClip({ id: 7 })]);
    api.toggleFavorite.mockResolvedValue(true);

    await clipboardList.refresh();
    api.getClips.mockClear();

    const star = parent.querySelector(".clip-star");
    star.click();
    await new Promise((r) => setTimeout(r, 0));

    expect(api.toggleFavorite).toHaveBeenCalledWith(7);
    expect(api.getClips).toHaveBeenCalled();
    expect(api.selectClip).not.toHaveBeenCalled();
  });

  it("点击非星区域走 selectClip", async () => {
    api.getClips.mockResolvedValueOnce([makeClip({ id: 9 })]);
    api.selectClip.mockResolvedValue(undefined);

    await clipboardList.refresh();

    const preview = parent.querySelector(".clip-preview");
    preview.click();
    await new Promise((r) => setTimeout(r, 0));

    expect(api.selectClip).toHaveBeenCalledWith(9);
    expect(api.toggleFavorite).not.toHaveBeenCalled();
  });
});
