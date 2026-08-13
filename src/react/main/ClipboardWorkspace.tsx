import { Clipboard, Search } from "lucide-react";
import { useEffect, useRef, useSyncExternalStore } from "react";
import { t } from "../shared/i18n";
import { clipboardStore } from "./clipboardStore";
import { ClipboardRow } from "./ClipboardRow";

export function ClipboardWorkspace() {
  const snapshot = useSyncExternalStore(
    clipboardStore.subscribe,
    clipboardStore.getSnapshot,
    clipboardStore.getSnapshot,
  );
  const searchRef = useRef<HTMLInputElement>(null);
  const items = snapshot.mode === "favorites" ? snapshot.favorites : snapshot.all;

  useEffect(() => {
    if (snapshot.searchVisible) searchRef.current?.focus();
  }, [snapshot.searchVisible]);

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
            snapshot={snapshot}
            onFocus={() => clipboardStore.pointerFocusRow(index)}
            onToggle={() => clipboardStore.toggleRowActions(clip, index)}
            onAction={(action, actionIndex) => {
              if (actionIndex >= 0) clipboardStore.focusAction(index, actionIndex);
              else clipboardStore.focusRow(index);
              void clipboardStore.invokeAction(clip, action);
            }}
          />
        ))}
      </main>

      <div id="empty-state" className="empty-state" hidden={items.length > 0}>
        <span className="empty-state-icon" aria-hidden="true"><Clipboard size={28} /></span>
        <span id="empty-state-text">
          {t(snapshot.mode === "favorites" ? "empty.favorites" : "empty.text")}
        </span>
      </div>

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
