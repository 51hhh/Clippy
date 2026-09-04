import { Clipboard, Search } from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { currentLocale } from "../../i18n/i18n.js";
import { onPasteFallback, type PasteOutcome } from "../../js/api.ts";
import { t } from "../shared/i18n";
import { clipboardStore } from "./clipboardStore";
import { ClipboardRow, type ClipboardRowHandlers } from "./ClipboardRow";
import { clipboardRowOffsets, clipboardVisibleRange } from "./clipboardVirtualization";

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
  const listRef = useRef<HTMLElement>(null);
  const [pasteFallback, setPasteFallback] = useState<PasteOutcome | null>(null);
  const [viewport, setViewport] = useState({ scrollTop: 0, height: 600 });
  const items = snapshot.mode === "favorites" ? snapshot.favorites : snapshot.all;
  const navigation = snapshot.navigation;
  const rowOffsets = useMemo(() => clipboardRowOffsets(items), [items]);
  const virtualRange = clipboardVisibleRange(
    rowOffsets,
    viewport.scrollTop,
    viewport.height,
  );
  // 语言是渲染的隐式输入（`t()` 不是 props 的函数），所以要显式喂给被 memo 的行，
  // 否则 `refreshLabels()` 之后行内的按钮文案不会跟着换。
  const locale = currentLocale();

  useEffect(() => {
    if (snapshot.searchVisible) searchRef.current?.focus();
  }, [snapshot.searchVisible]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onPasteFallback((outcome) => {
      if (!disposed) setPasteFallback(outcome);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const syncViewport = useCallback((element: HTMLElement) => {
    const next = { scrollTop: element.scrollTop, height: element.clientHeight || 600 };
    setViewport((current) => (
      current.scrollTop === next.scrollTop && current.height === next.height ? current : next
    ));
  }, []);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    syncViewport(list);
    if (typeof ResizeObserver === "undefined") {
      const update = () => syncViewport(list);
      window.addEventListener("resize", update);
      return () => window.removeEventListener("resize", update);
    }
    const observer = new ResizeObserver(() => syncViewport(list));
    observer.observe(list);
    return () => observer.disconnect();
  }, [syncViewport]);

  useEffect(() => {
    const list = listRef.current;
    const index = snapshot.navigation.focusedRow;
    if (!list || index < 0 || index + 1 >= rowOffsets.length) return;
    const rowTop = rowOffsets[index];
    const rowBottom = rowOffsets[index + 1];
    const viewportBottom = list.scrollTop + list.clientHeight;
    if (rowTop < list.scrollTop) list.scrollTop = rowTop;
    else if (rowBottom > viewportBottom) list.scrollTop = rowBottom - list.clientHeight;
    syncViewport(list);
    // 只在焦点、面板或查询切换时校正；分页追加不能把用户从列表底部拉回焦点行。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot.navigation.focusedRow, snapshot.mode, snapshot.query, syncViewport]);

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
        ref={listRef}
        id="clip-list"
        className="clip-list"
        role="listbox"
        aria-label={t("clipboard.history")}
        onScroll={(event) => {
          const element = event.currentTarget;
          syncViewport(element);
          if (element.scrollTop + element.clientHeight >= element.scrollHeight - 50) {
            void clipboardStore.loadMore();
          }
        }}
      >
        <div
          className="clip-list-virtual-content"
          role="presentation"
          style={{
            paddingTop: virtualRange.paddingTop,
            paddingBottom: virtualRange.paddingBottom,
          }}
        >
          {items.slice(virtualRange.start, virtualRange.end).map((clip, relativeIndex) => {
            const index = virtualRange.start + relativeIndex;
            return (
              <ClipboardRow
                key={clip.id}
                clip={clip}
                index={index}
                totalCount={items.length}
                focused={navigation.focusedRow === index}
                // 未获焦的行恒为 -1：否则在动作之间左右移动焦点会让每一行的 props 都变。
                focusedAction={navigation.focusedRow === index ? navigation.focusedCol : -1}
                expanded={navigation.expandedRow === clip.id}
                favoriteMode={snapshot.mode === "favorites"}
                locale={locale}
                handlers={ROW_HANDLERS}
              />
            );
          })}
        </div>
      </main>

      <div id="empty-state" className="empty-state" hidden={items.length > 0}>
        <span className="empty-state-icon" aria-hidden="true"><Clipboard size={28} /></span>
        <span id="empty-state-text">
          {t(snapshot.mode === "favorites" ? "empty.favorites" : "empty.text")}
        </span>
      </div>

      {pasteFallback && (
        <div className="paste-fallback" role="status" title={pasteFallback.detail ?? ""}>
          <span>
            {t("clipboard.pasteFallback")}
            {pasteFallback.reason_code ? ` (${pasteFallback.reason_code})` : ""}
          </span>
          <button type="button" onClick={() => setPasteFallback(null)} aria-label={t("action.dismiss")}>
            ×
          </button>
        </div>
      )}

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
