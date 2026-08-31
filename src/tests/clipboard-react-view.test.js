import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({ getClipThumbnail: vi.fn() }));

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

  it("keeps empty text empty and shows only size and time", () => {
    i18n.init("zh-CN");
    const html = renderToStaticMarkup(React.createElement(ClipboardRow, {
      clip: clip({ text_content: "" }),
      index: 0,
      snapshot: snapshot(),
      onFocus: vi.fn(),
      onToggle: vi.fn(),
      onAction: vi.fn(),
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
      index: 0,
      snapshot: snapshot(),
      onFocus: vi.fn(),
      onToggle: vi.fn(),
      onAction: vi.fn(),
    }));

    expect(html).not.toContain("clip-row-html-badge");
    expect(html).not.toContain("HTML");
    expect(html).toMatch(/clip-row-meta[^>]*>5 B ·/);
  });
});
