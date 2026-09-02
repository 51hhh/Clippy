import { Clipboard, FolderOpen, Search } from "lucide-react";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { currentLocale } from "../../i18n/i18n.js";
import { openPinImageDialog } from "../../js/api.ts";
import { t } from "../shared/i18n";
import { clipboardStore } from "./clipboardStore";
import { ClipboardRow, type ClipboardRowHandlers } from "./ClipboardRow";

/**
 * 行的回调表，模块级常量。
 *
 * 放在模块作用域是为了让引用**恒定不变**：`ClipboardRow` 被 `memo` 包着，
 * 每次渲染新建一份回调会让它每次都判定 props 变了、整份 memo 白做。
 * 所以行需要的 `clip`/`index` 由调用参数传进来，而不是靠闭包捕获。
 */
const ROW_HANDLERS: ClipboardRowHandlers = {
  onFocus: (index) => clipboardStore.pointerFocusRow(index),
  onToggle: (clip, index) => clipboardStore.toggleRowActions(clip, index),
  onAction: (clip, index, action, actionIndex) => {
    if (actionIndex >= 0) clipboardStore.focusAction(index, actionIndex);
    else clipboardStore.focusRow(index);
    void clipboardStore.invokeAction(clip, action);
  },
};

export function ClipboardWorkspace() {
  const snapshot = useSyncExternalStore(
    clipboardStore.subscribe,
    clipboardStore.getSnapshot,
    clipboardStore.getSnapshot,
  );
  const searchRef = useRef<HTMLInputElement>(null);
  const [openError, setOpenError] = useState(false);
  const items = snapshot.mode === "favorites" ? snapshot.favorites : snapshot.all;
  const navigation = snapshot.navigation;
  // 语言是渲染的隐式输入（`t()` 不是 props 的函数），所以要显式喂给被 memo 的行，
  // 否则 `refreshLabels()` 之后行内的按钮文案不会跟着换。
  const locale = currentLocale();

  useEffect(() => {
    if (snapshot.searchVisible) searchRef.current?.focus();
  }, [snapshot.searchVisible]);

  useEffect(() => {
    const onOpenError = () => setOpenError(true);
    window.addEventListener("pin-image-open-error", onOpenError);
    return () => window.removeEventListener("pin-image-open-error", onOpenError);
  }, []);

  useEffect(() => {
    const focused = document.querySelector<HTMLElement>(
      `.clip-row[data-idx="${snapshot.navigation.focusedRow}"]`,
    );
    focused?.scrollIntoView?.({ block: "nearest" });
  }, [snapshot.navigation.focusedRow, snapshot.mode]);

  return (
    <>
      <aside id="search-bar" className={`search-bar${snapshot.searchVisible ? " visible" : ""}`} hidden={!snapshot.searchVisible}>
        <span className="search-bar-icon" aria-hidden="true"><Search size={14} /></span>
        <input
          ref={searchRef}
          id="search-input"
          className="search-bar-input"
          type="text"
          value={snapshot.query}
          placeholder={t("search.placeholder")}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => clipboardStore.scheduleQuery(event.target.value)}
        />
        <span className="search-bar-hint">{t("search.escHint")}</span>
      </aside>

      <main
        id="clip-list"
        className="clip-list"
        role="listbox"
        aria-label={t("clipboard.history")}
        onScroll={(event) => {
          const element = event.currentTarget;
          if (element.scrollTop + element.clientHeight >= element.scrollHeight - 50) {
            void clipboardStore.loadMore();
          }
        }}
      >
        {items.map((clip, index) => (
          <ClipboardRow
            key={clip.id}
            clip={clip}
            index={index}
            focused={navigation.focusedRow === index}
            // 未获焦的行恒为 -1：否则在动作之间左右移动焦点会让每一行的 props 都变。
            focusedAction={navigation.focusedRow === index ? navigation.focusedCol : -1}
            expanded={navigation.expandedRow === clip.id}
            favoriteMode={snapshot.mode === "favorites"}
            locale={locale}
            handlers={ROW_HANDLERS}
          />
        ))}
      </main>

      <div id="empty-state" className="empty-state" hidden={items.length > 0}>
        <span className="empty-state-icon" aria-hidden="true"><Clipboard size={28} /></span>
        <span id="empty-state-text">
          {t(snapshot.mode === "favorites" ? "empty.favorites" : "empty.text")}
        </span>
      </div>

      <button
        type="button"
        className="open-image-button"
        title={t("pin.openImageShortcut")}
        onClick={() => {
          setOpenError(false);
          void openPinImageDialog().catch((reason) => {
            console.warn("Open image failed:", reason);
            setOpenError(true);
          });
        }}
      >
        <FolderOpen size={14} aria-hidden="true" />
        <span>{t("pin.openImage")}</span>
        <kbd>Ctrl+O</kbd>
      </button>
      {openError && <div className="open-image-error" role="alert">{t("pin.openImageFailed")}</div>}

      <footer id="segment-tabs" className="segment-tabs" role="tablist">
        <span className="segment-indicator" data-position={snapshot.mode === "favorites" ? "left" : "right"} />
        {(["favorites", "all"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            role="tab"
            className={`segment-tab${snapshot.mode === mode ? " active" : ""}`}
            aria-selected={snapshot.mode === mode}
            onClick={() => void clipboardStore.setPanelMode(mode)}
          >
            <span className="segment-tab-label">{t(mode === "all" ? "tabs.all" : "tabs.favorites")}</span>
            <span className="segment-tab-count">
              {mode === "all"
                ? snapshot.all.length
                : snapshot.favoritesLoaded
                  ? snapshot.favorites.length
                  : snapshot.all.filter((clip) => clip.is_favorite).length}
            </span>
          </button>
        ))}
      </footer>
    </>
  );
}
