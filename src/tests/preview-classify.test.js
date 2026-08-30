/**
 * preview-classify.test.js — 内容类型判定表
 *
 * 用户报的症状：同一条剪贴板记录在主栏显示 HTML、在右侧栏显示 YAML。
 * 根因是类型有两套标准（后端 content_type 只有 text/html/image，预览另跑内容嗅探）。
 * 现在主栏不显示类型，判定只剩这张表，因此这里锁两件事：
 *   1. 表的顺序（顺序即语义，JWT 必须早于可逆编码）
 *   2. 每条规则指向的渲染器真实存在（表和渲染器分开写，改名会静默失联）
 */
import { describe, expect, it, vi } from "vitest";

vi.mock("../js/api.ts", () => ({
  getClipImage: vi.fn(),
  ocrAvailable: vi.fn(),
  ocrImage: vi.fn(),
  getConfig: vi.fn(),
  fetchUrlMeta: vi.fn(),
  copyText: vi.fn(),
}));

import { CLASSIFY_RULES, classifyText } from "../js/preview/classify.js";
import { createPreviewRenderers } from "../js/preview/renderers.js";

const KINDS = [
  "url", "json", "jwt", "encoding", "hash", "encrypted", "color", "timestamp",
  "uuid", "ip", "email", "mac", "cron", "date", "semver", "number-base",
  "gradient", "data-size", "regex", "coordinate", "mime-type", "math",
  "http-status",
];

function availableRenderers() {
  return new Set(Object.keys(createPreviewRenderers({
    contentEl: document.createElement("div"),
    badgeEl: document.createElement("div"),
    metaEl: document.createElement("div"),
    getLibraries: () => ({}),
    isCurrentClip: () => true,
  })));
}

describe("类型判定表", () => {
  it("顺序固定，先匹配先赢", () => {
    expect(CLASSIFY_RULES.map((rule) => rule.kind)).toEqual(KINDS);
  });

  it("kind 不重复（表是唯一的类型来源，重名意味着后一条永远不会命中）", () => {
    expect(new Set(KINDS).size).toBe(KINDS.length);
  });

  it("每条规则指向的渲染器都存在", () => {
    const names = availableRenderers();
    // encoding 的渲染器由检测结果决定，两个分支都要在
    const declared = CLASSIFY_RULES.flatMap((rule) => (
      typeof rule.renderer === "function"
        ? [rule.renderer({ type: "base64-image" }), rule.renderer({ type: "base64" })]
        : [rule.renderer]
    ));
    for (const name of declared) {
      expect(names.has(name), name).toBe(true);
    }
  });

  it("只有 JSON 和 JWT 需要等延迟加载的库", () => {
    expect(CLASSIFY_RULES.filter((rule) => rule.needsLibs).map((rule) => rule.kind))
      .toEqual(["json", "jwt"]);
  });
});

describe("classifyText", () => {
  it("URL 走卡片，不需要 hljs", () => {
    const decision = classifyText("https://example.com/page");
    expect(decision).toMatchObject({ kind: "url", renderer: "renderUrlCard", needsLibs: false });
    expect(decision.args).toEqual(["https://example.com/page"]);
  });

  it("三段 Base64 认成 JWT 而不是编码内容", () => {
    const jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.fake-signature";
    expect(classifyText(jwt)).toMatchObject({ kind: "jwt", needsLibs: true });
  });

  it("检测结果透传给渲染器，不重算一遍", () => {
    const bcrypt = "$2y$10$N9qo8uLOickgx2ZMRZoMyeIjZAgcfl7p92ldGxad68LJZdL17lhWy";
    const decision = classifyText(bcrypt);
    expect(decision.kind).toBe("hash");
    // 哈希类型是 detect 的返回值，直接当第二个实参交给渲染器
    expect(decision.args).toEqual([bcrypt, "bcrypt"]);

    // 可逆编码同理：整个检测结果对象原样交给渲染器
    const encoded = classifyText("SGVsbG8gd29ybGQgZnJvbSBjbGlwcHk=");
    expect(encoded.kind).toBe("encoding");
    expect(encoded.args[0]).toMatchObject({ type: "base64", decoded: "Hello world from clippy" });
  });

  // 纯 hex 的哈希同时也是合法 base64 字符集，`encoding` 排在 `hash` 前面就会先认领它。
  // 这是重构之前就有的行为，这里明确锁住，改顺序时不会不知不觉动到渲染结果。
  it("纯 hex 的哈希仍旧先被可逆编码认领（既有行为）", () => {
    expect(classifyText("d41d8cd98f00b204e9800998ecf8427e").kind).toBe("encoding");
  });

  it("判不出来就交给异步尾段（Markdown / 代码高亮 / 富文本 / 纯文本）", () => {
    // YAML 只有 hljs 认得，表里判不出来才轮得到它
    expect(classifyText("name: clippy\nversion: 1\ndeps:\n  - tauri")).toBeNull();
    expect(classifyText("just a sentence of prose")).toBeNull();
    expect(classifyText("")).toBeNull();
  });
});
