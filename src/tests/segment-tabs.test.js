import { describe, it, expect, beforeEach } from "vitest";
import * as segmentTabs from "../js/segment-tabs.js";

function setup() {
  document.body.innerHTML = `<footer id="segment-tabs"></footer>`;
  const root = document.getElementById("segment-tabs");
  const changes = [];
  segmentTabs.init(root, (m) => changes.push(m));
  return { root, changes };
}

describe("segment-tabs", () => {
  it("初始渲染两个 tab，默认 all 高亮", () => {
    const { root } = setup();
    const tabs = root.querySelectorAll(".segment-tab");
    expect(tabs.length).toBe(2);
    expect(tabs[0].classList.contains("active")).toBe(true);
    expect(tabs[1].classList.contains("active")).toBe(false);
  });

  it("setCounts 更新计数", () => {
    const { root } = setup();
    segmentTabs.setCounts({ all: 12, favorites: 3 });
    const counts = root.querySelectorAll(".segment-tab-count");
    expect(counts[0].textContent).toBe("12");
    expect(counts[1].textContent).toBe("3");
  });

  it("点 favorites 切换并触发 onChange", () => {
    const { root, changes } = setup();
    root.querySelectorAll(".segment-tab")[1].click();
    expect(segmentTabs.getMode()).toBe("favorites");
    expect(changes).toEqual(["favorites"]);
  });
});
