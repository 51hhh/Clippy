const THEMES = [
  { id: "light", i18nKey: "settings.theme.light" },
  { id: "dark", i18nKey: "settings.theme.dark" },
  { id: "nord", i18nKey: "settings.theme.nord" },
  { id: "solarized-light", i18nKey: "settings.theme.solarizedLight" },
  { id: "rose", i18nKey: "settings.theme.rose" },
  { id: "midnight", i18nKey: "settings.theme.midnight" },
];

function appendPreviewRow(document, preview, { active = false, accent = false, short = false } = {}) {
  const row = document.createElement("div");
  row.className = active ? "tp-row tp-row-active" : "tp-row";

  const dot = document.createElement("span");
  dot.className = accent ? "tp-dot tp-dot-accent" : "tp-dot";

  const line = document.createElement("span");
  line.className = short
    ? "tp-line tp-line-short"
    : active
      ? "tp-line tp-line-strong"
      : "tp-line";

  row.append(dot, line);
  preview.appendChild(row);
}

/** 创建主题选择器并让模块独占其 DOM 与临时选择状态。 */
export function createThemePicker({ container, translate, persistTheme }) {
  const document = container.ownerDocument;
  let selectedTheme = "light";

  function applyTheme(theme) {
    document.documentElement.dataset.theme = theme;
  }

  function updateSelection() {
    for (const card of container.querySelectorAll(".theme-card")) {
      const selected = card.dataset.theme === selectedTheme;
      card.classList.toggle("selected", selected);
      card.setAttribute("aria-checked", String(selected));
    }
  }

  async function selectTheme(theme) {
    selectedTheme = theme;
    applyTheme(theme);
    updateSelection();
    try {
      await persistTheme(theme);
    } catch (error) {
      console.warn("主题持久化失败:", error);
    }
  }

  function createCard(theme) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "theme-card";
    card.dataset.theme = theme.id;
    card.setAttribute("role", "radio");

    const preview = document.createElement("div");
    preview.className = "theme-preview";
    preview.dataset.theme = theme.id;
    preview.setAttribute("aria-hidden", "true");

    const bar = document.createElement("div");
    bar.className = "tp-bar";
    preview.appendChild(bar);
    appendPreviewRow(document, preview);
    appendPreviewRow(document, preview, { active: true, accent: true });
    appendPreviewRow(document, preview, { short: true });

    const label = document.createElement("span");
    label.className = "theme-name";
    label.dataset.i18n = theme.i18nKey;
    label.textContent = translate(theme.i18nKey);

    card.append(preview, label);
    card.addEventListener("click", () => {
      void selectTheme(theme.id);
    });
    return card;
  }

  function render() {
    container.replaceChildren(...THEMES.map(createCard));
    updateSelection();
  }

  return {
    initialize(theme) {
      selectedTheme = theme || "light";
      applyTheme(selectedTheme);
      render();
    },
    refreshLabels: render,
    get value() {
      return selectedTheme;
    },
  };
}
