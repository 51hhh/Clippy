/**
 * codec-panel-dom.test.js — 编解码面板与自定义下拉框的接线
 *
 * 结构直接取自 index.html：面板改用 custom-select 之后，"选择操作 → 执行"、
 * "最近使用分组重建"、"反向操作"这几条链路都不再经过原生 <select>，需要单独兜住。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

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

describe("编解码面板（自定义下拉框）", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    mountCodecPanel();
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

  it("选过的操作进入最近使用分组并显示标题", async () => {
    setInput("Hello");
    option("sha256").click();
    await vi.waitFor(() => expect(output()).toHaveLength(64));

    const recent = document.getElementById("codec-recent");
    expect([...recent.children].map((item) => item.dataset.value)).toEqual(["sha256"]);
    expect(document.getElementById("codec-recent-group").hidden).toBe(false);
    expect(JSON.parse(localStorage.getItem("clippy-codec-recent"))).toEqual(["sha256"]);
  });

  it("最近使用为空时整个分组连标题一起隐藏", () => {
    expect(document.getElementById("codec-recent").children).toHaveLength(0);
    expect(document.getElementById("codec-recent-group").hidden).toBe(true);
  });

  it("最近使用最多 5 项且最新在前", async () => {
    const ops = ["md5", "sha1", "sha256", "sha512", "rot13", "url-encode"];
    setInput("Hello");
    for (const op of ops) {
      option(op).click();
      // eslint-disable-next-line no-await-in-loop
      await vi.waitFor(() => expect(document.getElementById("codec-select").dataset.value).toBe(op));
    }
    const recent = [...document.getElementById("codec-recent").children];
    expect(recent.map((item) => item.dataset.value)).toEqual(
      ["url-encode", "rot13", "sha512", "sha256", "sha1"],
    );
  });

  it("重建最近使用后选中态只保留一个", async () => {
    setInput("Hello");
    option("hex-encode").click();
    await vi.waitFor(() => expect(output()).toBe("48 65 6c 6c 6f"));
    expect(document.querySelectorAll("#codec-select .custom-select-option.selected")).toHaveLength(1);
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
});
