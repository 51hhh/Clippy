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

function clip(o = {}) {
  return {
    id: 1, text_content: "hello", is_favorite: false,
    byte_size: 5, created_at: Math.floor(Date.now() / 1000),
    content_type: "text", ...o,
  };
}

let counts;
let summons;

function setup() {
  document.body.innerHTML = `
    <main id="clip-list"></main>
    <div id="empty-state" hidden><span id="empty-state-text"></span></div>
  `;
  const listEl = document.getElementById("clip-list");
  const emptyEl = document.getElementById("empty-state");
  counts = [];
  summons = [];
  clipboardList.__test__.reset();
  clipboardList.init({
    listEl,
    emptyEl,
    onCountsChange: (c) => counts.push(c),
    onSummonSearch: (s) => summons.push(s),
  });
}

describe("clipboard-list 状态机", () => {
  beforeEach(() => {
    setup();
    telemetry.disable();
    api.getClips.mockReset();
    api.toggleFavorite.mockReset();
    api.selectClip.mockReset();
    api.deleteClip.mockReset();
  });

  afterEach(() => {
    clipboardList.__test__.reset();
  });

  it("默认 init 后 focusedRow=0、counts 上报", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 }), clip({ id: 2, is_favorite: true })]);
    await clipboardList.refresh();
    const s = clipboardList.__test__.state();
    expect(s.focusedRow).toBe(0);
    expect(s.focusedCol).toBe(-1);
    expect(s.expandedRow).toBeNull();
    expect(counts.at(-1)).toEqual({ all: 2, favorites: 1 });
  });

  it("第一行按 ↑ 召唤搜索", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 }), clip({ id: 2 })]);
    await clipboardList.refresh();
    clipboardList.moveRow(-1);
    expect(summons).toContain("keyboard");
  });

  it("↓ 移行；不溢出", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 }), clip({ id: 2 })]);
    await clipboardList.refresh();
    clipboardList.moveRow(1);
    expect(clipboardList.__test__.state().focusedRow).toBe(1);
    clipboardList.moveRow(1);
    expect(clipboardList.__test__.state().focusedRow).toBe(1);
  });

  it("expandRowActions → 行进入按钮区，focusedCol=0", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 })]);
    await clipboardList.refresh();
    clipboardList.expandRowActions();
    const s = clipboardList.__test__.state();
    expect(s.expandedRow).toBe(1);
    expect(s.focusedCol).toBe(0);
  });

  it("按钮区最左再 ← 收回", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 })]);
    await clipboardList.refresh();
    clipboardList.expandRowActions();
    clipboardList.moveCol(-1);
    expect(clipboardList.__test__.state().expandedRow).toBeNull();
    expect(clipboardList.__test__.state().focusedCol).toBe(-1);
  });

  it("按钮间右移最大到 2", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 })]);
    await clipboardList.refresh();
    clipboardList.expandRowActions();
    clipboardList.moveCol(1);
    expect(clipboardList.__test__.state().focusedCol).toBe(1);
    clipboardList.moveCol(1);
    expect(clipboardList.__test__.state().focusedCol).toBe(2);
    clipboardList.moveCol(1);
    expect(clipboardList.__test__.state().focusedCol).toBe(2);
  });

  it("activateFocus 在行体 = selectClip", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 7 })]);
    api.selectClip.mockResolvedValue(undefined);
    await clipboardList.refresh();
    await clipboardList.activateFocus("keyboard");
    expect(api.selectClip).toHaveBeenCalledWith(7);
  });

  it("activateFocus 在按钮 0 = selectClip；在按钮 1 = toggleFavorite", async () => {
    api.getClips.mockResolvedValue([clip({ id: 9 })]);
    api.selectClip.mockResolvedValue(undefined);
    api.toggleFavorite.mockResolvedValue(true);

    await clipboardList.refresh();
    clipboardList.expandRowActions(); // focusedCol=0
    await clipboardList.activateFocus("keyboard");
    expect(api.selectClip).toHaveBeenCalledWith(9);

    clipboardList.moveCol(1); // focusedCol=1
    await clipboardList.activateFocus("keyboard");
    expect(api.toggleFavorite).toHaveBeenCalledWith(9);
  });

  it("删除二次确认：第一次按 ✕ 不调 deleteClip；第二次才调", async () => {
    api.getClips.mockResolvedValue([clip({ id: 5 })]);
    api.deleteClip.mockResolvedValue(undefined);
    await clipboardList.refresh();
    clipboardList.expandRowActions();
    clipboardList.moveCol(1);
    clipboardList.moveCol(1); // focusedCol=2 (delete)

    await clipboardList.activateFocus("keyboard");
    expect(api.deleteClip).not.toHaveBeenCalled();
    expect(clipboardList.__test__.state().deletePending).not.toBeNull();

    await clipboardList.activateFocus("keyboard");
    expect(api.deleteClip).toHaveBeenCalledWith(5);
  });

  it("setPanelMode favorites 时只展示收藏", async () => {
    api.getClips.mockResolvedValue([
      clip({ id: 1, is_favorite: false }),
      clip({ id: 2, is_favorite: true }),
    ]);
    await clipboardList.refresh();
    clipboardList.setPanelMode("favorites");
    const rows = document.querySelectorAll(".clip-row");
    expect(rows.length).toBe(1);
    expect(rows[0].dataset.id).toBe("2");
  });

  it("空列表显示 empty-state", async () => {
    api.getClips.mockResolvedValueOnce([]);
    await clipboardList.refresh();
    expect(document.getElementById("empty-state").hidden).toBe(false);
  });

  it("行点击 = 复制并触发 selectClip", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 11 })]);
    api.selectClip.mockResolvedValue(undefined);
    await clipboardList.refresh();
    document.querySelector(".clip-row").click();
    await new Promise((r) => setTimeout(r, 0));
    expect(api.selectClip).toHaveBeenCalledWith(11);
  });

  it("点 ⋯ 触发器展开按钮组", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 13 })]);
    await clipboardList.refresh();
    document.querySelector(".clip-row-trigger").click();
    expect(clipboardList.__test__.state().expandedRow).toBe(13);
  });

  it("canExpandHere/hasExpanded 切换", async () => {
    api.getClips.mockResolvedValueOnce([clip({ id: 1 })]);
    await clipboardList.refresh();
    expect(clipboardList.canExpandHere()).toBe(true);
    expect(clipboardList.hasExpanded()).toBe(false);
    clipboardList.expandRowActions();
    expect(clipboardList.canExpandHere()).toBe(false);
    expect(clipboardList.hasExpanded()).toBe(true);
  });
});
