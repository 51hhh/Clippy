import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClipThumbnail: vi.fn(),
  openPinImageDialog: vi.fn(),
  deleteClip: vi.fn(),
  getClips: vi.fn(),
  onPasteFallback: vi.fn(),
  selectClip: vi.fn(),
  toggleFavorite: vi.fn(),
}));

import * as i18n from "../i18n/i18n.js";
import { getClipThumbnail, onPasteFallback } from "../js/api.ts";
import { ClipboardRow } from "../react/main/ClipboardRow.tsx";
import { ClipboardWorkspace } from "../react/main/ClipboardWorkspace.tsx";
import { clipboardStore } from "../react/main/clipboardStore.ts";

/**
 * 行的 props 是拍扁的标量（不是整个 snapshot），这样它能被 `memo` 挡住——
 * 一次焦点移动只重渲失焦和获焦那两行，而不是全部 30 行。
 */
function props(overrides = {}) {
  return {
    index: 0,
    focused: true,
    focusedAction: -1,
    expanded: false,
    favoriteMode: false,
    locale: "en",
    handlers: { onFocus: vi.fn(), onToggle: vi.fn(), onAction: vi.fn() },
    ...overrides,
  };
}

function clip(overrides = {}) {
  return {
    id: 1,
    content_type: "text",
    text_content: "hello",
    html_content: null,
    image_data: null,
    content_hash: "hash",
    is_favorite: false,
    is_sensitive: false,
    created_at: Math.floor(Date.now() / 1000),
    byte_size: 5,
    ...overrides,
  };
}

describe("React clipboard row", () => {
  beforeEach(() => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    i18n.init("en");
  });

  afterEach(() => {
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  it("escapes user content and exposes accessible actions", () => {
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ text_content: '<img src=x onerror="alert(1)">' }),
      ...props(),
    }));

    expect(html).toContain("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;");
    expect(html).not.toContain("<img src=x");
    expect(html).toContain('aria-label="Copy"');
    expect(html).toContain('role="option"');
  });

  it("exposes a discoverable open-image entry with its keyboard hint", () => {
    const html = renderToStaticMarkup(React.createElement(ClipboardWorkspace));
    expect(html).toContain("Open image");
    expect(html).toContain("Ctrl+O");
    expect(html).toContain("open-image-button");
  });

  it("keeps empty text empty and shows only size and time", () => {
    i18n.init("zh-CN");
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ text_content: "" }),
      ...props(),
    }));

    expect(html).toContain("5 B");
    expect(html).not.toContain("富文本]");
    // 类型不再出现在列表行：它只在预览面板的 badge 上，那里按内容判定
    expect(html).not.toContain("文本 ·");
  });

  // 用户看到的症状：同一条 HTML 片段在主栏写 HTML、在右侧栏写 YAML。
  // 主栏彻底不显示类型（既没有 badge 也不进 meta），矛盾才不会再出现。
  it("never labels a row with a content type", () => {
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ content_type: "html", text_content: "key: value" }),
      ...props(),
    }));

    expect(html).not.toContain("clip-row-html-badge");
    expect(html).not.toContain("HTML");
    expect(html).toMatch(/clip-row-meta[^>]*>5 B ·/);
  });

  it("loads image thumbnails near the viewport and releases them offscreen", async () => {
    let deliverIntersection;
    let observedRow;
    let observerOptions;
    const disconnect = vi.fn();
    class FakeIntersectionObserver {
      constructor(callback, options) {
        deliverIntersection = (isIntersecting) => callback([{ isIntersecting }]);
        observerOptions = options;
      }

      observe(row) {
        observedRow = row;
      }

      disconnect() {
        disconnect();
      }
    }
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    vi.mocked(getClipThumbnail).mockResolvedValue("thumbnail-bytes");

    const container = document.createElement("div");
    container.className = "clip-list";
    document.body.append(container);
    const root = createRoot(container);

    try {
      await act(async () => root.render(React.createElement(ClipboardRow, {
        clip: clip({ id: 42, content_type: "image", byte_size: 1024 }),
        ...props(),
      })));

      const row = container.querySelector(".clip-row");
      expect(observedRow).toBe(row);
      expect(observerOptions).toEqual({ root: container, rootMargin: "160px 0px" });
      expect(getClipThumbnail).not.toHaveBeenCalled();

      await act(async () => deliverIntersection(true));
      expect(getClipThumbnail).toHaveBeenCalledOnce();
      expect(getClipThumbnail).toHaveBeenCalledWith(42);
      expect(container.querySelector(".clip-row-thumb-img")?.getAttribute("src"))
        .toBe("data:image/png;base64,thumbnail-bytes");

      await act(async () => deliverIntersection(false));
      expect(container.querySelector(".clip-row-thumb-img")).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
      vi.unstubAllGlobals();
    }

    expect(disconnect).toHaveBeenCalledOnce();
  });

  it("mounts only a small window for ten thousand clipboard rows", async () => {
    const originalSnapshot = clipboardStore.getSnapshot();
    const items = Array.from({ length: 10_000 }, (_, index) => clip({
      id: index + 1,
      text_content: `clip ${index + 1}`,
    }));
    clipboardStore.snapshot = {
      ...originalSnapshot,
      all: items,
      navigation: { ...originalSnapshot.navigation, focusedRow: 0 },
      revision: originalSnapshot.revision + 1,
    };
    vi.mocked(onPasteFallback).mockResolvedValue(() => undefined);
    vi.stubGlobal("ResizeObserver", class {
      observe() {}
      disconnect() {}
    });

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    try {
      await act(async () => root.render(React.createElement(ClipboardWorkspace)));
      const list = container.querySelector(".clip-list");
      expect(list.querySelectorAll(".clip-row").length).toBeLessThan(20);
      expect(list.querySelector(".clip-row")?.dataset.idx).toBe("0");
      expect(list.querySelector(".clip-row")?.getAttribute("aria-setsize")).toBe("10000");
      expect(list.querySelector(".clip-row")?.getAttribute("aria-posinset")).toBe("1");

      Object.defineProperty(list, "clientHeight", { configurable: true, value: 600 });
      Object.defineProperty(list, "scrollHeight", { configurable: true, value: 770_008 });
      list.scrollTop = 385_000;
      await act(async () => list.dispatchEvent(new Event("scroll", { bubbles: true })));

      expect(list.querySelectorAll(".clip-row").length).toBeLessThan(20);
      expect(Number(list.querySelector(".clip-row")?.dataset.idx)).toBeGreaterThan(4_900);

      await act(async () => clipboardStore.focusRow(9_999));
      const focused = list.querySelector('.clip-row.focused[data-idx="9999"]');
      expect(focused).not.toBeNull();
      expect(focused?.getAttribute("aria-posinset")).toBe("10000");
      expect(list.scrollTop).toBe(769_400);
    } finally {
      await act(async () => root.unmount());
      clipboardStore.snapshot = originalSnapshot;
      container.remove();
      vi.unstubAllGlobals();
    }
  });
});
