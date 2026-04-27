import { describe, it, expect, beforeEach } from "vitest";
import * as searchBar from "../js/search-bar.js";

function setup() {
  document.body.innerHTML = `
    <aside id="search-bar" class="search-bar" hidden>
      <span class="search-bar-icon" aria-hidden="true"><svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg></span>
      <input class="search-bar-input" type="text"/>
      <span class="search-bar-hint">Esc</span>
    </aside>
  `;
  const root = document.getElementById("search-bar");
  const queries = [];
  searchBar.init(root, (q) => queries.push(q));
  return { root, queries };
}

describe("search-bar", () => {
  beforeEach(() => { searchBar.hide(); });

  it("默认隐藏；summon 显示并聚焦", () => {
    const { root } = setup();
    expect(searchBar.isVisible()).toBe(false);
    expect(root.hidden).toBe(true);
    searchBar.summon("keyboard");
    expect(searchBar.isVisible()).toBe(true);
    expect(root.hidden).toBe(false);
    expect(document.activeElement.classList.contains("search-bar-input")).toBe(true);
  });

  it("Esc 三段：清空 → 隐藏 → 返回 panel", () => {
    setup();
    expect(searchBar.dismissStage()).toBe("panel");

    searchBar.summon();
    const input = document.querySelector(".search-bar-input");
    input.value = "abc";
    expect(searchBar.dismissStage()).toBe("clear");
    expect(input.value).toBe("");
    expect(searchBar.dismissStage()).toBe("hide");
    expect(searchBar.isVisible()).toBe(false);
  });

  it("输入触发 onQuery（防抖 200ms）", async () => {
    const { queries } = setup();
    searchBar.summon();
    const input = document.querySelector(".search-bar-input");
    input.value = "foo";
    input.dispatchEvent(new Event("input"));
    await new Promise((r) => setTimeout(r, 220));
    expect(queries).toContain("foo");
  });
});
