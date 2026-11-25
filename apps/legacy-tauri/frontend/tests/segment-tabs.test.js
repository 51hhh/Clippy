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
  it("初始渲染两个 tab，顺序 [Favorites, All]，默认 all 高亮", () => {
    const { root } = setup();
    const tabs = root.querySelectorAll(".segment-tab");
    expect(tabs.length).toBe(2);
    expect(tabs[0].dataset.mode).toBe("favorites");
    expect(tabs[1].dataset.mode).toBe("all");
    expect(tabs[1].classList.contains("active")).toBe(true);
    expect(tabs[0].classList.contains("active")).toBe(false);
  });

  it("setCounts 更新计数", () => {
    const { root } = setup();
    segmentTabs.setCounts({ all: 12, favorites: 3 });
    const tabs = root.querySelectorAll(".segment-tab");
    // tabs[0]=favorites, tabs[1]=all
    expect(tabs[0].querySelector(".segment-tab-count").textContent).toBe("3");
    expect(tabs[1].querySelector(".segment-tab-count").textContent).toBe("12");
  });

  it("点 favorites 切换并触发 onChange，indicator 滑到 left", () => {
    const { root, changes } = setup();
    root.querySelector('.segment-tab[data-mode="favorites"]').click();
    expect(segmentTabs.getMode()).toBe("favorites");
    expect(changes).toEqual(["favorites"]);
    expect(root.querySelector(".segment-indicator").dataset.position).toBe("left");
  });
});
