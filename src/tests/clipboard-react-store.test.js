import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClips: vi.fn(),
  deleteClip: vi.fn(),
  selectClip: vi.fn(),
  toggleFavorite: vi.fn(),
}));

import * as api from "../js/api.ts";
import { ClipboardStore } from "../react/main/clipboardStore.ts";

function clip(id, favorite = false) {
  return {
    id,
    content_type: "text",
    text_content: `clip ${id}`,
    html_content: null,
    image_data: null,
    content_hash: String(id),
    is_favorite: favorite,
    is_sensitive: false,
    created_at: 1,
    byte_size: 6,
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

describe("React clipboard store", () => {
  let store;

  beforeEach(() => {
    store = new ClipboardStore();
    api.getClips.mockReset();
    api.deleteClip.mockReset();
    api.selectClip.mockReset();
    api.toggleFavorite.mockReset();
  });

  it("loads clips, focuses the first row and invokes copy", async () => {
    const focusChanges = [];
    store.initialize({ onFocusChange: (value) => focusChanges.push(value?.id || null) });
    api.getClips.mockResolvedValueOnce([clip(1), clip(2, true)]);
    api.selectClip.mockResolvedValue(undefined);

    await store.refresh();
    expect(store.getSnapshot().navigation.focusedRow).toBe(0);
    expect(focusChanges.at(-1)).toBe(1);
    await store.activateFocus();
    expect(api.selectClip).toHaveBeenCalledWith(1);
  });

  it("loads an authoritative empty favorites result", async () => {
    api.getClips.mockResolvedValueOnce([clip(1, true)]);
    await store.refresh();
    api.getClips.mockResolvedValueOnce([]);
    await store.setPanelMode("favorites");

    expect(store.getSnapshot()).toMatchObject({
      mode: "favorites",
      favorites: [],
      favoritesLoaded: true,
    });
  });

  it("does not let a stale query replace the latest response", async () => {
    const first = deferred();
    const second = deferred();
    api.getClips.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const older = store.setQuery("old");
    const newer = store.setQuery("new");
    second.resolve([clip(2)]);
    first.resolve([clip(1)]);
    await Promise.all([older, newer]);

    expect(store.getSnapshot().query).toBe("new");
    expect(store.getSnapshot().all.map((item) => item.id)).toEqual([2]);
  });

  it("does not append stale pagination after the query changes", async () => {
    const page = deferred();
    api.getClips
      .mockResolvedValueOnce(Array.from({ length: 30 }, (_, index) => clip(index + 1)))
      .mockReturnValueOnce(page.promise);
    await store.refresh();

    const loading = store.loadMore();
    store.scheduleQuery("new");
    page.resolve([clip(31)]);
    await loading;

    expect(store.getSnapshot().query).toBe("new");
    expect(store.getSnapshot().all).toHaveLength(30);
    expect(store.getSnapshot().loadingMore).toBe(false);
  });

  it("does not restore released items from pending pagination", async () => {
    const page = deferred();
    api.getClips
      .mockResolvedValueOnce(Array.from({ length: 30 }, (_, index) => clip(index + 1)))
      .mockReturnValueOnce(page.promise);
    await store.refresh();

    const loading = store.loadMore();
    store.releaseMemory();
    page.resolve([clip(31)]);
    await loading;

    expect(store.getSnapshot().all).toEqual([]);
    expect(store.getSnapshot().loadingMore).toBe(false);
  });

  it("allows pagination again after refresh cancels a pending page", async () => {
    const page = deferred();
    const refreshed = deferred();
    api.getClips
      .mockResolvedValueOnce(Array.from({ length: 30 }, (_, index) => clip(index + 1)))
      .mockReturnValueOnce(page.promise)
      .mockReturnValueOnce(refreshed.promise);
    await store.refresh();

    const loading = store.loadMore();
    const refreshing = store.refresh();
    refreshed.resolve(Array.from({ length: 30 }, (_, index) => clip(index + 101)));
    page.resolve([clip(31)]);
    await Promise.all([loading, refreshing]);

    expect(store.getSnapshot().all[0].id).toBe(101);
    expect(store.getSnapshot().loadingMore).toBe(false);
  });

  it("keeps the latest panel choice while favorites are loading", async () => {
    const favorites = deferred();
    api.getClips.mockResolvedValueOnce([clip(1)]).mockReturnValueOnce(favorites.promise);
    await store.refresh();

    const showFavorites = store.setPanelMode("favorites");
    await store.setPanelMode("all");
    favorites.resolve([clip(2, true)]);
    await showFavorites;

    expect(store.getSnapshot().mode).toBe("all");
    expect(store.getFocusedClip()?.id).toBe(1);
  });

  it("keeps prepend ordering and removes entries from both views", async () => {
    api.getClips.mockResolvedValueOnce([clip(1), clip(2, true)]);
    await store.refresh();

    store.prependClip(clip(2, true));
    expect(store.getSnapshot().all.map((item) => item.id)).toEqual([2, 1]);
    store.removeClip(2);
    expect(store.getSnapshot().all.map((item) => item.id)).toEqual([1]);
    expect(store.getSnapshot().favorites).toEqual([]);
  });

  it("does not move favorites focus for a non-favorite clipboard event", async () => {
    api.getClips.mockResolvedValueOnce([clip(1, true)]);
    await store.refresh();
    api.getClips.mockResolvedValueOnce([clip(1, true)]);
    await store.setPanelMode("favorites");

    store.prependClip(clip(2, false));
    expect(store.getSnapshot().navigation.focusedRow).toBe(0);
    expect(store.getFocusedClip()?.id).toBe(1);
  });

  it("summons and dismisses search in clear, hide and panel stages", async () => {
    store.summonSearch();
    await store.setQuery("needle");
    expect(store.dismissSearchStage()).toBe("clear");
    await Promise.resolve();
    expect(store.getSnapshot().query).toBe("");
    expect(store.dismissSearchStage()).toBe("hide");
    expect(store.dismissSearchStage()).toBe("panel");
  });

  it("ignores the first pointer move after keyboard navigation", async () => {
    api.getClips.mockResolvedValueOnce([clip(1), clip(2)]);
    await store.refresh();
    store.moveRow(1);
    expect(store.getFocusedClip()?.id).toBe(2);

    store.pointerFocusRow(0);
    expect(store.getFocusedClip()?.id).toBe(2);
    store.pointerFocusRow(0);
    expect(store.getFocusedClip()?.id).toBe(1);
  });
});
