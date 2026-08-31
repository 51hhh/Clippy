/**
 * 设置页分页。
 *
 * 设置项已经多到一屏放不下（快捷键、主题、截图、翻译、统计……），平铺时找一项要滚很久。
 * 分页只切换 `hidden`，不销毁任何面板——各控制器在装配时就抓住了元素引用，
 * 面板一旦被移除或重建，它们持有的引用就会失效。
 */

const STORAGE_KEY = "clippy.settings.tab";

/** 记住上次停留的分页。localStorage 在无痕/受限环境下会抛异常，读写都要兜住。 */
function readStoredTab() {
  try {
    return window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoredTab(key) {
  try {
    window.localStorage.setItem(STORAGE_KEY, key);
  } catch {
    // 记不住就每次从第一页开始，不影响功能。
  }
}

export function initSettingsTabs(root = document) {
  const tablist = root.querySelector("#settings-tabs");
  if (!tablist) return null;
  const tabs = [...tablist.querySelectorAll("[data-settings-tab]")];
  const panels = new Map(
    [...root.querySelectorAll("[data-settings-panel]")].map((panel) => [
      panel.dataset.settingsPanel,
      panel,
    ]),
  );
  if (!tabs.length || !panels.size) return null;

  function activate(key, { focus = false } = {}) {
    if (!panels.has(key)) return;
    for (const tab of tabs) {
      const selected = tab.dataset.settingsTab === key;
      tab.setAttribute("aria-selected", String(selected));
      // roving tabindex：只有当前分页进 Tab 序列，方向键负责在分页间移动。
      tab.tabIndex = selected ? 0 : -1;
      tab.classList.toggle("active", selected);
      if (selected && focus) tab.focus();
    }
    for (const [panelKey, panel] of panels) {
      panel.hidden = panelKey !== key;
    }
    writeStoredTab(key);
  }

  function moveFocus(fromIndex, delta) {
    const next = (fromIndex + delta + tabs.length) % tabs.length;
    activate(tabs[next].dataset.settingsTab, { focus: true });
  }

  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => activate(tab.dataset.settingsTab));
    tab.addEventListener("keydown", (event) => {
      switch (event.key) {
        case "ArrowLeft":
        case "ArrowUp":
          event.preventDefault();
          moveFocus(index, -1);
          break;
        case "ArrowRight":
        case "ArrowDown":
          event.preventDefault();
          moveFocus(index, 1);
          break;
        case "Home":
          event.preventDefault();
          activate(tabs[0].dataset.settingsTab, { focus: true });
          break;
        case "End":
          event.preventDefault();
          activate(tabs[tabs.length - 1].dataset.settingsTab, { focus: true });
          break;
        default:
          break;
      }
    });
  });

  const stored = readStoredTab();
  activate(panels.has(stored) ? stored : tabs[0].dataset.settingsTab);

  return { activate };
}
