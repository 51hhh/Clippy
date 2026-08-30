/**
 * codec-panel-dom.test.js — 编解码面板与自定义下拉框的接线
 *
 * 结构直接取自 index.html：面板改用 custom-select 之后，"选择操作 → 执行"、
 * "收藏分组重建"、"反向操作"这几条链路都不再经过原生 <select>，需要单独兜住。
 * 另外锁定面板文案跟随界面语言（专有名词不翻译）。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as i18n from "../i18n/i18n.js";

const setCodecVisible = vi.fn(async () => {});
const copyText = vi.fn(async () => {});
vi.mock("../js/api.ts", () => ({
  get copyText() { return copyText; },
  get setCodecVisible() { return setCodecVisible; },
}));

const { init, setInput, __test__ } = await import("../js/codec.js");
const REVERSE_BASE64_DECODE = __test__.REVERSE_MAP["base64-decode"];

/** 把 index.html 里真实的 #codec-panel 搬进测试文档，避免 fixture 与产品分叉 */
function mountCodecPanel() {
  const markup = readFileSync(resolve(process.cwd(), "index.html"), "utf8");
  const parsed = new DOMParser().parseFromString(markup, "text/html");
  const panel = parsed.getElementById("codec-panel");
  document.body.replaceChildren(document.adoptNode(panel));
  return panel;
}

function option(value) {
  return document.querySelector(`#codec-select .custom-select-option[data-value="${value}"]`);
}

function output() {
  return document.getElementById("codec-output").textContent;
}

function favoriteButton() {
  return document.getElementById("codec-favorite");
}

describe("编解码面板（自定义下拉框）", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    mountCodecPanel();
    i18n.init("en");
    init();
  });

  it("默认操作来自标记为 selected 的选项", () => {
    expect(document.getElementById("codec-select").dataset.value).toBe("base64-encode");
    expect(document.querySelector(".custom-select-value").textContent).toBe("Base64 Encode");
  });

  it("点击选项即执行对应操作", async () => {
    setInput("SGVsbG8=");
    option("base64-decode").click();
    await vi.waitFor(() => expect(output()).toBe("Hello"));
  });

  it("星星按钮收藏当前操作并写入分组", async () => {
    setInput("Hello");
    option("sha256").click();
    await vi.waitFor(() => expect(output()).toHaveLength(64));
    favoriteButton().click();

    const favorites = document.getElementById("codec-favorites");
    expect([...favorites.children].map((item) => item.dataset.value)).toEqual(["sha256"]);
    expect(document.getElementById("codec-favorites-group").hidden).toBe(false);
    expect(JSON.parse(localStorage.getItem("clippy-codec-favorites"))).toEqual(["sha256"]);
  });

  it("再点一次星星取消收藏，分组连标题一起隐藏", async () => {
    setInput("Hello");
    option("md5").click();
    await vi.waitFor(() => expect(output()).toHaveLength(32));
    favoriteButton().click();
    favoriteButton().click();

    expect(document.getElementById("codec-favorites").children).toHaveLength(0);
    expect(document.getElementById("codec-favorites-group").hidden).toBe(true);
    expect(JSON.parse(localStorage.getItem("clippy-codec-favorites"))).toEqual([]);
  });

  it("收藏状态由两个不同星星图标表示", () => {
    // 未收藏是描边星星（fill="none"），收藏后换成实心星星
    expect(favoriteButton().getAttribute("aria-pressed")).toBe("false");
    expect(favoriteButton().innerHTML).toContain('fill="none"');
    expect(favoriteButton().classList.contains("is-favorite")).toBe(false);

    favoriteButton().click();
    expect(favoriteButton().getAttribute("aria-pressed")).toBe("true");
    expect(favoriteButton().innerHTML).toContain('fill="currentColor"');
    expect(favoriteButton().classList.contains("is-favorite")).toBe(true);
  });

  it("切换操作时星星跟着当前操作的收藏状态", async () => {
    favoriteButton().click(); // 收藏默认的 base64-encode
    setInput("Hello");
    option("rot13").click();
    await vi.waitFor(() => expect(output()).toBe("Uryyb"));
    expect(favoriteButton().getAttribute("aria-pressed")).toBe("false");

    option("base64-encode").click();
    await vi.waitFor(() => expect(favoriteButton().getAttribute("aria-pressed")).toBe("true"));
  });

  it("空收藏时整个分组连标题一起隐藏", () => {
    expect(document.getElementById("codec-favorites").children).toHaveLength(0);
    expect(document.getElementById("codec-favorites-group").hidden).toBe(true);
  });

  it("重建收藏分组后选中态只保留一个", async () => {
    favoriteButton().click();
    setInput("Hello");
    option("hex-encode").click();
    await vi.waitFor(() => expect(output()).toBe("48 65 6c 6c 6f"));
    favoriteButton().click();
    expect(document.querySelectorAll("#codec-select .custom-select-option.selected")).toHaveLength(1);
  });

  it("收藏分组里的副本点了也执行", async () => {
    setInput("Hello");
    option("hex-encode").click();
    await vi.waitFor(() => expect(output()).toBe("48 65 6c 6c 6f"));
    favoriteButton().click();

    option("base64-encode").click();
    await vi.waitFor(() => expect(output()).toBe("SGVsbG8="));
    // 收藏分组在 DOM 里排在最前，副本必须走同一条委托出口
    const copy = document.querySelector('#codec-favorites .custom-select-option[data-value="hex-encode"]');
    copy.click();
    await vi.waitFor(() => expect(output()).toBe("48 65 6c 6c 6f"));
  });

  it("⇄ 按钮切换到反向操作并重新执行", async () => {
    setInput("SGVsbG8=");
    option("base64-decode").click();
    await vi.waitFor(() => expect(output()).toBe("Hello"));

    document.getElementById("codec-swap-dir").click();
    expect(document.getElementById("codec-select").dataset.value).toBe(REVERSE_BASE64_DECODE);
    // 输入没变，方向反过来：对 "SGVsbG8=" 再编码一次
    await vi.waitFor(() => expect(output()).toBe("U0dWc2JHOD0="));
  });

  it("快速切换操作时异步结果不会覆盖最新一次", async () => {
    setInput("Hello");
    option("sha512").click();
    option("hex-encode").click();
    await vi.waitFor(() => expect(output()).toBe("48 65 6c 6c 6f"));
    // 再等一拍，确认后到的 sha512 结果被丢弃
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(output()).toBe("48 65 6c 6c 6f");
  });

  it("智能提示点击后套用建议的操作", async () => {
    setInput("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig");
    const hint = document.getElementById("codec-smart-hint");
    await vi.waitFor(() => expect(hint.hidden).toBe(false));
    hint.click();
    expect(document.getElementById("codec-select").dataset.value).toBe("jwt-decode");
    await vi.waitFor(() => expect(output()).toContain("=== Payload ==="));
  });

  it("智能提示带上建议操作的显示名", async () => {
    setInput("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig");
    const hint = document.getElementById("codec-smart-hint");
    await vi.waitFor(() => expect(hint.hidden).toBe(false));
    expect(document.getElementById("codec-hint-text").textContent)
      .toBe("Detected JWT Decode — click to apply");
    // 灯泡表情已去掉，提示只剩文本
    expect(hint.querySelector(".codec-hint-icon")).toBeNull();
  });
});

describe("编解码面板的界面语言", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    mountCodecPanel();
  });

  it("中文下操作名与按钮提示都跟随语言，专有名词保留原文", () => {
    i18n.init("zh-CN");
    init();

    expect(option("base64-decode").textContent).toBe("Base64 解码");
    expect(option("html-encode").textContent).toBe("HTML 实体编码");
    expect(option("num-base").textContent).toBe("进制转换");
    expect(option("sha256").textContent).toBe("SHA-256");
    // 触发按钮显示的是当前操作，init 之后必须已经是中文
    expect(document.querySelector(".custom-select-value").textContent).toBe("Base64 编码");
    expect(document.querySelector("#codec-favorites-group .custom-select-group-title").textContent)
      .toBe("收藏");
    expect(document.getElementById("codec-swap-dir").title).toBe("反向操作");
    expect(document.getElementById("codec-copy").title).toBe("复制结果");
    expect(document.getElementById("codec-input").placeholder).toBe("输入…");
    expect(favoriteButton().title).toBe("加入收藏");
  });

  it("收藏分组里的副本与错误提示同样是中文", async () => {
    i18n.init("zh-CN");
    init();
    favoriteButton().click();

    expect(document.querySelector("#codec-favorites .custom-select-option").textContent)
      .toBe("Base64 编码");
    expect(favoriteButton().title).toBe("取消收藏");

    option("jwt-decode").click();
    setInput("not-a-jwt");
    await vi.waitFor(() => expect(output()).toBe("错误：JWT 格式无效"));
  });

  it("语言切换后 refreshLabels 补上 JS 写入的文案", async () => {
    i18n.init("en");
    init();
    favoriteButton().click();
    expect(document.querySelector(".custom-select-value").textContent).toBe("Base64 Encode");

    // 真实链路是 config-changed → i18n.init(applyToDOM) → codec.refreshLabels()
    i18n.init("zh-CN");
    const { refreshLabels } = await import("../js/codec.js");
    refreshLabels();

    expect(document.querySelector(".custom-select-value").textContent).toBe("Base64 编码");
    expect(document.querySelector("#codec-favorites .custom-select-option").textContent)
      .toBe("Base64 编码");
    expect(favoriteButton().title).toBe("取消收藏");
  });
});
