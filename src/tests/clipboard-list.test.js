import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as telemetry from "../js/telemetry.js";
import {
  consumePointerMove,
  createNavigationState,
  expandActions,
  moveColumnFocus,
  moveRowFocus,
  normalizeAfterRefresh,
} from "../js/clipboard/navigation-state.js";
import {
  formatRelativeTime,
  formatSize,
  formatType,
} from "../js/clipboard/formatters.js";
import { syncClipboardRow } from "../js/clipboard/row-renderer.js";

vi.mock("../js/api.ts", () => ({
  getClips: vi.fn(),
  deleteClip: vi.fn(),
  toggleFavorite: vi.fn(),
  selectClip: vi.fn(),
  getClipImage: vi.fn(),
}));

import * as api from "../js/api.ts";
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
  const listEl = document.createElement("main");
  listEl.id = "clip-list";
  const emptyEl = document.createElement("div");
  emptyEl.id = "empty-state";
  emptyEl.hidden = true;
  const emptyText = document.createElement("span");
  emptyText.id = "empty-state-text";
  emptyEl.appendChild(emptyText);
  document.body.replaceChildren(listEl, emptyEl);
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
    api.getClipImage.mockReset();
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

  it("删除按钮直接调用 deleteClip", async () => {
    api.getClips.mockResolvedValue([clip({ id: 5 })]);
    api.deleteClip.mockResolvedValue(undefined);
    await clipboardList.refresh();
    clipboardList.expandRowActions();
    clipboardList.moveCol(1);
    clipboardList.moveCol(1); // focusedCol=2 (delete)

    await clipboardList.activateFocus("keyboard");
    expect(api.deleteClip).toHaveBeenCalledWith(5);
  });

  it("setPanelMode favorites 时只展示收藏", async () => {
    api.getClips.mockResolvedValue([
      clip({ id: 1, is_favorite: false }),
      clip({ id: 2, is_favorite: true }),
    ]);
    await clipboardList.refresh();
    // 收藏模式独立查询
    api.getClips.mockResolvedValueOnce([
      clip({ id: 2, is_favorite: true }),
    ]);
    await clipboardList.setPanelMode("favorites");
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

  it("用户文本只作为文本节点渲染", async () => {
    api.getClips.mockResolvedValueOnce([
      clip({ text_content: '<img src=x onerror="globalThis.hacked=true">' }),
    ]);
    await clipboardList.refresh();

    const preview = document.querySelector(".clip-row-preview");
    expect(preview.textContent).toContain("<img src=x");
    expect(preview.querySelector("img")).toBeNull();
    expect(globalThis.hacked).toBeUndefined();
  });

  it("图片缩略图通过 api wrapper 异步加载", async () => {
    api.getClips.mockResolvedValueOnce([
      clip({ id: 21, content_type: "image", text_content: null }),
    ]);
    api.getClipImage.mockResolvedValueOnce("cG5n");
    await clipboardList.refresh();
    await vi.waitFor(() => {
      expect(document.querySelector(".clip-row-thumb-img")?.src)
        .toBe("data:image/png;base64,cG5n");
    });
    expect(api.getClipImage).toHaveBeenCalledWith(21);
  });
});

describe("clipboard 导航纯状态机", () => {
  it("刷新后将初始焦点收敛到第一行", () => {
    expect(normalizeAfterRefresh(createNavigationState(), 3)).toMatchObject({
      focusedRow: 0,
      focusedCol: -1,
      expandedRow: null,
    });
    expect(normalizeAfterRefresh(createNavigationState(), 0).focusedRow).toBe(-1);
  });

  it("竖向移动收起动作区并启用一次鼠标保护", () => {
    const expanded = expandActions(
      { ...createNavigationState(), focusedRow: 0 },
      7,
    );
    const transition = moveRowFocus(expanded, 1, 3);
    expect(transition).toMatchObject({
      summonSearch: false,
      nextState: {
        focusedRow: 1,
        focusedCol: -1,
        expandedRow: null,
        keyboardNav: true,
      },
    });

    const firstPointerMove = consumePointerMove(transition.nextState);
    expect(firstPointerMove.ignore).toBe(true);
    expect(consumePointerMove(firstPointerMove.nextState).ignore).toBe(false);
  });

  it("第一行向上请求搜索，行体横移请求切换面板", () => {
    const state = { ...createNavigationState(), focusedRow: 0 };
    expect(moveRowFocus(state, -1, 2).summonSearch).toBe(true);
    expect(moveColumnFocus(state, -1, 2, "all").requestedMode).toBe("favorites");
    expect(moveColumnFocus(state, 1, 2, "favorites").requestedMode).toBe("all");
  });

  it("收藏模式的动作按钮按视觉方向反转", () => {
    const expanded = expandActions(
      { ...createNavigationState(), focusedRow: 0 },
      9,
    );
    expect(moveColumnFocus(expanded, -1, 1, "favorites").nextState.focusedCol).toBe(1);
    expect(moveColumnFocus(expanded, 1, 1, "favorites").nextState).toMatchObject({
      focusedCol: -1,
      expandedRow: null,
    });
  });
});

describe("clipboard 差量行渲染", () => {
  it("相同条目序列切换收藏面板时同步布局 class", () => {
    const row = document.createElement("article");
    row.className = "clip-row favorites-mode";
    const trigger = document.createElement("button");
    trigger.className = "clip-row-trigger";
    const actions = document.createElement("div");
    actions.className = "clip-row-actions";
    const favorite = document.createElement("button");
    favorite.dataset.action = "favorite";
    actions.appendChild(favorite);
    const main = document.createElement("div");
    main.className = "clip-row-main";
    row.append(trigger, actions, main);

    syncClipboardRow(
      row,
      clip({ id: 4, is_favorite: true }),
      0,
      { focusedRow: 0, focusedCol: -1, expandedRow: null },
      "all",
    );
    expect(row.classList.contains("favorites-mode")).toBe(false);
    expect([...row.children]).toEqual([main, actions, trigger]);

    syncClipboardRow(
      row,
      clip({ id: 4, is_favorite: true }),
      0,
      { focusedRow: 0, focusedCol: -1, expandedRow: null },
      "favorites",
    );
    expect(row.classList.contains("favorites-mode")).toBe(true);
    expect([...row.children]).toEqual([trigger, actions, main]);
  });
});

describe("clipboard 展示格式", () => {
  const translate = (key, params = {}) => `${key}:${params.n ?? ""}`;
  const now = Date.UTC(2026, 7, 11, 12, 0, 0);

  it("格式化字节边界与内容类型", () => {
    expect(formatSize(1023)).toBe("1023 B");
    expect(formatSize(1024)).toBe("1.0 KB");
    expect(formatSize(1024 * 1024)).toBe("1.0 MB");
    expect(formatType(null)).toBe("Text");
    expect(formatType("html")).toBe("HTML");
    expect(formatType("custom")).toBe("custom");
  });

  it.each([
    [30, "time.justNow:"],
    [5 * 60, "time.minutesAgo:5"],
    [3 * 60 * 60, "time.hoursAgo:3"],
    [24 * 60 * 60, "time.yesterday:"],
    [4 * 24 * 60 * 60, "time.daysAgo:4"],
  ])("按固定 now 格式化相对时间", (elapsedSeconds, expected) => {
    expect(formatRelativeTime((now - elapsedSeconds * 1000) / 1000, { now, translate }))
      .toBe(expected);
  });
});
