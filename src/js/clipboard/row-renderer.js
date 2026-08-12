import { t } from "../../i18n/i18n.js";
import * as icons from "../icons.js";
import { formatRelativeTime, formatSize, formatType } from "./formatters.js";

/**
 * 创建一行剪贴板 DOM。所有业务动作由回调交还 facade 处理。
 */
export function createClipboardRow({
  clip,
  index,
  navigation,
  panelMode,
  thumbnailCache,
  maxThumbnailCache,
  loadThumbnail,
  onAction,
  onToggleActions,
  onPointerFocus,
}) {
  const row = document.createElement("article");
  row.className = "clip-row";
  row.classList.toggle("focused", index === navigation.focusedRow);
  row.classList.toggle("favorite", !!clip.is_favorite);
  row.classList.toggle("expanded", navigation.expandedRow === clip.id);
  row.classList.toggle("favorites-mode", panelMode === "favorites");
  row.dataset.idx = index;
  row.dataset.id = clip.id;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", String(index === navigation.focusedRow));

  const main = document.createElement("div");
  main.className = "clip-row-main";

  const preview = document.createElement("div");
  preview.className = "clip-row-preview";
  renderPreview(preview, clip, thumbnailCache, maxThumbnailCache, loadThumbnail);

  const meta = document.createElement("div");
  meta.className = "clip-row-meta";
  const metaParts = [
    `${formatRelativeTime(clip.created_at, { translate: t })} · ${formatType(clip.content_type)} · ${formatSize(clip.byte_size)}`,
  ];
  if (clip.is_sensitive) {
    row.classList.add("sensitive");
    metaParts.unshift("🔒");
  }
  meta.textContent = metaParts.join(" ");
  main.append(preview, meta);

  const actions = createActionGroup(clip, index, navigation, onAction);
  const trigger = createActionTrigger(clip, index, onToggleActions);
  if (panelMode === "favorites") row.append(trigger, actions, main);
  else row.append(main, actions, trigger);

  row.addEventListener("click", () => onAction(clip, "copy", index, -1));
  row.addEventListener("mousemove", () => onPointerFocus(clip, index, row));
  return row;
}

/** 现有 id 序列未变化时，只同步行状态，避免重建 DOM。 */
export function syncClipboardRow(row, clip, index, navigation) {
  const focused = index === navigation.focusedRow;
  row.classList.toggle("focused", focused);
  row.classList.toggle("expanded", navigation.expandedRow === clip.id);
  row.classList.toggle("favorite", !!clip.is_favorite);
  row.setAttribute("aria-selected", String(focused));

  const favoriteButton = row.querySelector('[data-action="favorite"]');
  if (favoriteButton) updateFavoriteButton(favoriteButton, clip.is_favorite);
  row.querySelectorAll(".clip-row-action").forEach((button, actionIndex) => {
    button.classList.toggle(
      "focused",
      focused && navigation.focusedCol === actionIndex,
    );
  });
}

function renderPreview(preview, clip, thumbnailCache, maxThumbnailCache, loadThumbnail) {
  if (clip.content_type === "image") {
    preview.classList.add("clip-row-preview--image");
    preview.appendChild(createThumbnail(clip.id, thumbnailCache, maxThumbnailCache, loadThumbnail));
    return;
  }
  if (clip.content_type === "html") {
    preview.textContent = (clip.text_content || t("preview.richText")).slice(0, 200);
    return;
  }
  preview.textContent = (clip.text_content || "").slice(0, 200);
}

function createThumbnail(clipId, cache, maxCacheSize, loadThumbnail) {
  const thumb = document.createElement("div");
  thumb.className = "clip-row-thumb";
  thumb.textContent = "🖼";

  const cached = cache.get(clipId);
  if (cached) {
    showThumbnailImage(thumb, cached);
    return thumb;
  }

  loadThumbnail(clipId)
    .then((base64) => {
      if (!base64) return;
      if (cache.size >= maxCacheSize) cache.delete(cache.keys().next().value);
      cache.set(clipId, base64);
      showThumbnailImage(thumb, base64);
    })
    .catch(() => {});
  return thumb;
}

function showThumbnailImage(thumb, base64) {
  const image = document.createElement("img");
  image.src = `data:image/png;base64,${base64}`;
  image.alt = "image";
  image.className = "clip-row-thumb-img";
  image.draggable = false;
  thumb.replaceChildren(image);
}

function createActionGroup(clip, rowIndex, navigation, onAction) {
  const actions = document.createElement("div");
  actions.className = "clip-row-actions";
  actionDefinitions(clip).forEach((action, actionIndex) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "clip-row-action";
    button.classList.toggle("is-favorite", action.key === "favorite" && !!clip.is_favorite);
    button.classList.toggle(
      "focused",
      rowIndex === navigation.focusedRow && navigation.focusedCol === actionIndex,
    );
    button.dataset.action = action.key;
    button.setAttribute("aria-label", action.label);
    button.title = action.label;

    const icon = document.createElement("span");
    icon.className = "clip-row-action-icon";
    icon.appendChild(parseStaticSvg(action.icon));
    button.appendChild(icon);
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      onAction(clip, action.key, rowIndex, actionIndex);
    });
    actions.appendChild(button);
  });
  return actions;
}

function createActionTrigger(clip, rowIndex, onToggleActions) {
  const trigger = document.createElement("button");
  trigger.type = "button";
  trigger.className = "clip-row-trigger";
  trigger.setAttribute("aria-label", t("action.more"));
  trigger.title = t("action.more");
  trigger.appendChild(parseStaticSvg(icons.more));
  trigger.addEventListener("click", (event) => {
    event.stopPropagation();
    onToggleActions(clip, rowIndex);
  });
  return trigger;
}

function updateFavoriteButton(button, isFavorite) {
  button.classList.toggle("is-favorite", !!isFavorite);
  const label = isFavorite ? t("action.unfavorite") : t("action.favorite");
  button.setAttribute("aria-label", label);
  button.title = label;
  const icon = button.querySelector(".clip-row-action-icon");
  if (icon) icon.replaceChildren(parseStaticSvg(isFavorite ? icons.starFill : icons.star));
}

function actionDefinitions(clip) {
  return [
    { key: "copy", label: t("action.copy"), icon: icons.copy },
    {
      key: "favorite",
      label: clip.is_favorite ? t("action.unfavorite") : t("action.favorite"),
      icon: clip.is_favorite ? icons.starFill : icons.star,
    },
    { key: "delete", label: t("action.delete"), icon: icons.trash },
  ];
}

/** SVG 字符串均来自本地 icons.js，不接收剪贴板或其他用户输入。 */
function parseStaticSvg(markup) {
  const parsed = new DOMParser().parseFromString(markup, "image/svg+xml");
  return document.importNode(parsed.documentElement, true);
}
