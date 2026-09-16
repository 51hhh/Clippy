import {
  deleteClip,
  getClips,
  selectClip,
  toggleFavorite,
  type ClipItem,
} from "../../js/api.ts";
import {
  collapseActions,
  consumePointerMove,
  createNavigationState,
  expandActions,
  focusAction,
  focusRowBody,
  moveColumnFocus,
  moveRowFocus,
  normalizeAfterRefresh,
  releaseNavigation,
  resetForPanelChange,
} from "../../js/clipboard/navigation-state.js";

export type PanelMode = "all" | "favorites";
type Navigation = ReturnType<typeof createNavigationState>;

export type ClipboardSnapshot = {
  all: ClipItem[];
  favorites: ClipItem[];
  mode: PanelMode;
  query: string;
  searchVisible: boolean;
  navigation: Navigation;
  dirty: boolean;
  loadingMore: boolean;
  favoritesLoaded: boolean;
  actionError: "copy" | "favorite" | "delete" | null;
  revision: number;
};

type Callbacks = {
  onFocusChange?: (clip: ClipItem | null) => void;
  onSummonSearch?: () => void;
};

const PAGE_SIZE = 30;

export class ClipboardStore {
  private snapshot: ClipboardSnapshot = {
    all: [],
    favorites: [],
    mode: "all",
    query: "",
    searchVisible: false,
    navigation: createNavigationState(),
    dirty: false,
    loadingMore: false,
    favoritesLoaded: false,
    actionError: null,
    revision: 0,
  };
  private listeners = new Set<() => void>();
  private callbacks: Callbacks = {};
  private allHasMore = true;
  private favoritesHasMore = true;
  private dataRevision = 0;
  private cacheIdentity: Record<PanelMode, { query: string; revision: number } | null> = { all: null, favorites: null };
  private loadedLimit: Record<PanelMode, number> = { all: PAGE_SIZE, favorites: PAGE_SIZE };
  private released = false;
  private requeryScheduled = false;
  private requestGeneration = 0;
  private queryTimer: number | null = null;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): ClipboardSnapshot => this.snapshot;

  initialize(callbacks: Callbacks): void {
    this.callbacks = callbacks;
  }

  private commit(update: Partial<ClipboardSnapshot>, notifyFocus = false): void {
    this.snapshot = {
      ...this.snapshot,
      ...update,
      revision: this.snapshot.revision + 1,
    };
    this.listeners.forEach((listener) => listener());
    if (notifyFocus) this.notifyFocus();
  }

  private visibleItems(snapshot = this.snapshot): ClipItem[] {
    return snapshot.mode === "favorites" ? snapshot.favorites : snapshot.all;
  }

  private notifyFocus(): void {
    this.callbacks.onFocusChange?.(
      this.visibleItems()[this.snapshot.navigation.focusedRow] || null,
    );
  }

  private cacheCurrent(mode = this.snapshot.mode): boolean {
    const identity = this.cacheIdentity[mode];
    return identity?.query === this.snapshot.query && identity.revision === this.dataRevision;
  }

  private navigationFor(items: ClipItem[], previousId = this.getFocusedClip()?.id): Navigation {
    const index = items.findIndex((item) => item.id === previousId);
    return normalizeAfterRefresh({
      ...this.snapshot.navigation,
      focusedRow: index >= 0 ? index : this.snapshot.navigation.focusedRow,
    }, items.length);
  }

  async refresh(): Promise<void> {
    this.released = false;
    await this.queryRange(PAGE_SIZE);
  }

  /** 数据事件后重新查询已加载的连续区间，旧 OFFSET 页不能接到新数据版本上。 */
  private async queryRange(limit: number): Promise<void> {
    const generation = ++this.requestGeneration;
    const revision = this.dataRevision;
    const { query, mode } = this.snapshot;
    const isCurrent = () => generation === this.requestGeneration
      && revision === this.dataRevision && !this.released;
    this.commit({ loadingMore: false });
    try {
      const items = await getClips(query || null, mode === "favorites", 0, limit);
      if (!isCurrent()) return;
      this.cacheIdentity[mode] = { query, revision };
      this.loadedLimit[mode] = limit;
      if (mode === "all") this.allHasMore = items.length >= limit;
      else this.favoritesHasMore = items.length >= limit;
      this.commit({
        [mode]: items,
        dirty: false,
        loadingMore: false,
        favoritesLoaded: mode === "favorites" || this.snapshot.favoritesLoaded,
        navigation: this.navigationFor(items),
      }, true);
    } catch (error) {
      if (!isCurrent()) return;
      console.error("Clipboard query failed", error);
      // 失败不能把缓存冒充新版本；再次聚焦或分页时仍可重试。
      this.commit({ dirty: true, loadingMore: false });
    }
  }

  private changeQuery(query: string): void {
    this.requestGeneration += 1;
    this.cacheIdentity = { all: null, favorites: null };
    this.loadedLimit = { all: PAGE_SIZE, favorites: PAGE_SIZE };
    this.commit({ query, all: [], favorites: [], favoritesLoaded: false,
      dirty: true, loadingMore: false, navigation: releaseNavigation(this.snapshot.navigation) }, true);
  }

  scheduleQuery(query: string): void {
    this.changeQuery(query);
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = window.setTimeout(() => {
      this.queryTimer = null;
      void this.refresh();
    }, 200);
  }

  async setQuery(query: string): Promise<void> {
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = null;
    this.changeQuery(query);
    await this.refresh();
  }

  getFocusedClip(): ClipItem | null {
    return this.visibleItems()[this.snapshot.navigation.focusedRow] || null;
  }

  getLatestClip(): ClipItem | null {
    return this.snapshot.all[0] || null;
  }

  async setPanelMode(mode: PanelMode): Promise<void> {
    if (mode === this.snapshot.mode) return;
    this.requestGeneration += 1;
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = null;
    const cached = this.cacheIdentity[mode];
    const items = cached?.query === this.snapshot.query
      ? (mode === "favorites" ? this.snapshot.favorites : this.snapshot.all) : [];
    this.commit({
      mode, [mode]: items, loadingMore: false, dirty: !this.cacheCurrent(mode),
      navigation: normalizeAfterRefresh(resetForPanelChange(this.snapshot.navigation), items.length),
    }, true);
    if (!this.cacheCurrent(mode)) await this.queryRange(this.loadedLimit[mode]);
  }

  getPanelMode(): PanelMode {
    return this.snapshot.mode;
  }

  prependClip(clip: ClipItem): void {
    this.invalidateData();
    // 隐藏后不重新持有缩略图；搜索匹配由后端的 FTS/LIKE 规则决定。
    if (this.released || this.snapshot.query) return;
    // 原条目只更新位置时，保持当前选中 ID。
    const previousFocus = this.getFocusedClip();
    const all = this.snapshot.all.filter((item) => item.id !== clip.id);
    all.unshift(clip);
    const favorites = clip.is_favorite
      ? [clip, ...this.snapshot.favorites.filter((item) => item.id !== clip.id)]
      : this.snapshot.favorites.filter((item) => item.id !== clip.id);
    const visibleChanged = this.snapshot.mode === "all" || clip.is_favorite;
    const visible = this.snapshot.mode === "favorites" ? favorites : all;
    const focusIndex = !visibleChanged
      ? this.snapshot.navigation.focusedRow
      : previousFocus
        ? Math.max(0, visible.findIndex((item) => item.id === previousFocus.id))
        : 0;
    this.commit({
      all,
      favorites,
      dirty: true,
      navigation: { ...this.snapshot.navigation, focusedRow: focusIndex },
    }, true);
  }

  removeClip(id: number): void {
    const previousId = this.getFocusedClip()?.id;
    this.invalidateData();
    const all = this.snapshot.all.filter((item) => item.id !== id);
    const favorites = this.snapshot.favorites.filter((item) => item.id !== id);
    const items = this.snapshot.mode === "favorites" ? favorites : all;
    this.commit({
      all,
      favorites,
      navigation: this.navigationFor(items, previousId),
    }, true);
  }

  moveRow(delta: number): void {
    const transition = moveRowFocus(
      this.snapshot.navigation,
      delta,
      this.visibleItems().length,
    );
    if (transition.summonSearch) {
      this.summonSearch();
      return;
    }
    this.commit({ navigation: transition.nextState }, true);
  }

  moveCol(delta: number): void {
    const transition = moveColumnFocus(
      this.snapshot.navigation,
      delta,
      this.visibleItems().length,
      this.snapshot.mode,
    );
    if (transition.requestedMode) {
      void this.setPanelMode(transition.requestedMode as PanelMode);
      return;
    }
    this.commit({ navigation: transition.nextState });
  }

  expandRowActions(): void {
    const clip = this.getFocusedClip();
    if (!clip) return;
    this.commit({ navigation: expandActions(this.snapshot.navigation, clip.id) });
  }

  collapseActions(): void {
    this.commit({ navigation: collapseActions(this.snapshot.navigation) });
  }

  canExpandHere(): boolean {
    return this.snapshot.navigation.focusedCol === -1
      && this.snapshot.navigation.expandedRow === null;
  }

  hasExpanded(): boolean {
    return this.snapshot.navigation.expandedRow !== null;
  }

  focusRow(index: number): void {
    this.commit({ navigation: focusRowBody(this.snapshot.navigation, index) }, true);
  }

  pointerFocusRow(index: number): void {
    const transition = consumePointerMove(this.snapshot.navigation);
    if (transition.ignore) {
      this.commit({ navigation: transition.nextState });
      return;
    }
    if (this.snapshot.navigation.focusedRow !== index) this.focusRow(index);
  }

  focusAction(index: number, actionIndex: number): void {
    this.commit({ navigation: focusAction(this.snapshot.navigation, index, actionIndex) }, true);
  }

  toggleRowActions(clip: ClipItem, index: number): void {
    const focused = focusRowBody(this.snapshot.navigation, index);
    const navigation = this.snapshot.navigation.expandedRow === clip.id
      ? collapseActions(focused)
      : expandActions(focused, clip.id);
    this.commit({ navigation }, true);
  }

  async invokeAction(clip: ClipItem, action: "copy" | "favorite" | "delete"): Promise<boolean> {
    this.commit({ actionError: null });
    try {
      if (action === "copy") await selectClip(clip.id);
      else if (action === "favorite") {
        await toggleFavorite(clip.id);
        this.invalidateData(false);
        await this.queryRange(this.loadedLimit[this.snapshot.mode]);
      } else {
        await deleteClip(clip.id);
        this.removeClip(clip.id);
      }
      return true;
    } catch (error) {
      console.error("Clipboard action failed", error);
      this.commit({ actionError: action });
      return false;
    }
  }

  dismissActionError(): void {
    this.commit({ actionError: null });
  }

  async activateFocus(): Promise<void> {
    const clip = this.getFocusedClip();
    if (!clip) return;
    const action = this.snapshot.navigation.focusedCol === -1
      ? "copy"
      : (["copy", "favorite", "delete"] as const)[this.snapshot.navigation.focusedCol];
    if (action) await this.invokeAction(clip, action);
  }

  async selectByIndex(index: number): Promise<boolean> {
    const clip = this.visibleItems()[index];
    if (!clip) return false;
    return this.invokeAction(clip, "copy");
  }

  async loadMore(): Promise<void> {
    if (this.snapshot.loadingMore || this.released) return;
    if (!this.cacheCurrent()) {
      await this.queryRange(this.loadedLimit[this.snapshot.mode]);
      return;
    }
    const favoritesMode = this.snapshot.mode === "favorites";
    if (!(favoritesMode ? this.favoritesHasMore : this.allHasMore)) return;
    const generation = this.requestGeneration;
    const query = this.snapshot.query;
    this.commit({ loadingMore: true });
    const current = this.visibleItems();
    try {
      const more = await getClips(
        query || null,
        favoritesMode,
        current.length,
        PAGE_SIZE,
      );
      if (
        generation !== this.requestGeneration
        || query !== this.snapshot.query
        || favoritesMode !== (this.snapshot.mode === "favorites")
      ) {
        if (generation === this.requestGeneration) this.commit({ loadingMore: false });
        return;
      }
      this.loadedLimit[this.snapshot.mode] = current.length + PAGE_SIZE;
      if (favoritesMode) {
        this.favoritesHasMore = more.length >= PAGE_SIZE;
        this.commit({ favorites: [...this.snapshot.favorites, ...more], loadingMore: false });
      } else {
        this.allHasMore = more.length >= PAGE_SIZE;
        this.commit({ all: [...this.snapshot.all, ...more], loadingMore: false });
      }
    } catch (error) {
      if (
        generation !== this.requestGeneration
        || query !== this.snapshot.query
        || favoritesMode !== (this.snapshot.mode === "favorites")
      ) return;
      console.error("Clipboard pagination failed", error);
      this.commit({ loadingMore: false });
    }
  }

  summonSearch(): void {
    if (!this.snapshot.searchVisible) {
      this.commit({ searchVisible: true });
      this.callbacks.onSummonSearch?.();
    }
  }

  dismissSearchStage(): "clear" | "hide" | "panel" {
    if (!this.snapshot.searchVisible) return "panel";
    if (this.snapshot.query) {
      void this.setQuery("");
      return "clear";
    }
    this.commit({ searchVisible: false });
    return "hide";
  }

  isSearchVisible(): boolean {
    return this.snapshot.searchVisible;
  }

  releaseMemory(): void {
    this.released = true;
    this.cacheIdentity = { all: null, favorites: null };
    this.loadedLimit = { all: PAGE_SIZE, favorites: PAGE_SIZE };
    this.requestGeneration += 1;
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = null;
    this.commit({
      all: [],
      favorites: [],
      favoritesLoaded: false,
      dirty: true,
      loadingMore: false,
      navigation: releaseNavigation(this.snapshot.navigation),
    });
  }

  private invalidateData(requery = true): void {
    this.dataRevision += 1;
    this.requestGeneration += 1;
    this.commit({ dirty: true, loadingMore: false });
    if (!requery || this.released || this.requeryScheduled || this.queryTimer !== null) return;
    this.requeryScheduled = true;
    queueMicrotask(() => {
      this.requeryScheduled = false;
      if (!this.released && this.snapshot.dirty && this.queryTimer === null) {
        void this.queryRange(this.loadedLimit[this.snapshot.mode]);
      }
    });
  }

  markDirty(): void {
    this.invalidateData();
  }

  isDirty(): boolean {
    return this.snapshot.dirty;
  }

  restoreRender(): void {
    const items = this.visibleItems();
    if (!items.length) return;
    this.commit({
      navigation: normalizeAfterRefresh(resetForPanelChange(this.snapshot.navigation), items.length),
    }, true);
  }

  refreshLabels(): void {
    this.commit({});
  }
}

export const clipboardStore = new ClipboardStore();
