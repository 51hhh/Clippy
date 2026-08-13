import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({ getClipImage: vi.fn() }));

import * as i18n from "../i18n/i18n.js";
import { ClipboardRow } from "../react/main/ClipboardRow.tsx";

function snapshot() {
  return {
    all: [],
    favorites: [],
    mode: "all",
    query: "",
    searchVisible: false,
    navigation: { focusedRow: 0, focusedCol: -1, expandedRow: null, keyboardNav: false },
    dirty: false,
    loadingMore: false,
    favoritesLoaded: false,
    revision: 0,
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
  beforeEach(() => i18n.init("en"));

  it("escapes user content and exposes accessible actions", () => {
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ text_content: '<img src=x onerror="alert(1)">' }),
      index: 0,
      snapshot: snapshot(),
      onFocus: vi.fn(),
      onToggle: vi.fn(),
      onAction: vi.fn(),
    }));

    expect(html).toContain("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;");
    expect(html).not.toContain("<img src=x");
    expect(html).toContain('aria-label="Copy"');
    expect(html).toContain('role="option"');
  });

  it("keeps empty text empty and localizes metadata type", () => {
    i18n.init("zh-CN");
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ text_content: "" }),
      index: 0,
      snapshot: snapshot(),
      onFocus: vi.fn(),
      onToggle: vi.fn(),
      onAction: vi.fn(),
    }));

    expect(html).toContain("文本 · 5 B");
    expect(html).not.toContain("富文本]");
  });
});
