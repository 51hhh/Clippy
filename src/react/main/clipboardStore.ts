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
    revision: 0,
  };
  private listeners = new Set<() => void>();
  private callbacks: Callbacks = {};
  private allHasMore = true;
  private favoritesHasMore = true;
  private favoritesDirty = true;
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

  async refresh(): Promise<void> {
    const generation = ++this.requestGeneration;
    try {
      const all = await getClips(this.snapshot.query || null, false, 0, PAGE_SIZE);
      if (generation !== this.requestGeneration) return;
      this.allHasMore = all.length >= PAGE_SIZE;
      this.favoritesDirty = true;
      let favorites = this.snapshot.favorites;
      if (this.snapshot.mode === "favorites") {
        favorites = await this.loadFavorites(generation);
        if (generation !== this.requestGeneration) return;
      }
      const items = this.snapshot.mode === "favorites" ? favorites : all;
      this.commit({
        all,
        favorites,
        dirty: false,
        loadingMore: false,
        favoritesLoaded: this.snapshot.mode === "favorites" || this.snapshot.favoritesLoaded,
        navigation: normalizeAfterRefresh(this.snapshot.navigation, items.length),
      }, true);
    } catch (error) {
      if (generation !== this.requestGeneration) return;
      console.error("Clipboard query failed", error);
      this.commit({
        all: [],
        navigation: normalizeAfterRefresh(this.snapshot.navigation, 0),
        dirty: false,
        loadingMore: false,
      }, true);
    }
  }

  private async loadFavorites(generation = this.requestGeneration): Promise<ClipItem[]> {
    try {
      const favorites = await getClips(this.snapshot.query || null, true, 0, PAGE_SIZE);
      if (generation !== this.requestGeneration) return this.snapshot.favorites;
      this.favoritesHasMore = favorites.length >= PAGE_SIZE;
      this.favoritesDirty = false;
      return favorites;
    } catch (error) {
      console.error("Favorites query failed", error);
      return [];
    }
  }

  scheduleQuery(query: string): void {
    this.requestGeneration += 1;
    this.commit({ query, loadingMore: false });
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = window.setTimeout(() => {
      this.queryTimer = null;
      void this.refresh();
    }, 200);
  }

  async setQuery(query: string): Promise<void> {
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = null;
    this.commit({ query, loadingMore: false });
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
    const generation = ++this.requestGeneration;
    const query = this.snapshot.query;
    let favorites = this.snapshot.favorites;
    const items = mode === "favorites" ? favorites : this.snapshot.all;
    this.commit({
      mode,
      favorites,
      loadingMore: false,
      favoritesLoaded: mode === "favorites" || this.snapshot.favoritesLoaded,
      navigation: normalizeAfterRefresh(resetForPanelChange(this.snapshot.navigation), items.length),
    }, true);
    if (mode !== "favorites" || !this.favoritesDirty) return;

    favorites = await this.loadFavorites(generation);
    if (
      generation !== this.requestGeneration
      || this.snapshot.mode !== "favorites"
      || this.snapshot.query !== query
    ) return;
    this.commit({
      favorites,
      navigation: normalizeAfterRefresh(this.snapshot.navigation, favorites.length),
    }, true);
  }

  getPanelMode(): PanelMode {
    return this.snapshot.mode;
  }

  prependClip(clip: ClipItem): void {
    const all = this.snapshot.all.filter((item) => item.id !== clip.id);
    all.unshift(clip);
    const favorites = clip.is_favorite
      ? [clip, ...this.snapshot.favorites.filter((item) => item.id !== clip.id)]
      : this.snapshot.favorites.filter((item) => item.id !== clip.id);
    const visibleChanged = this.snapshot.mode === "all" || clip.is_favorite;
    const focusIndex = !visibleChanged
      ? this.snapshot.navigation.focusedRow
      : this.getFocusedClip()?.id === clip.id
        ? 0
        : this.snapshot.navigation.focusedRow >= 0
          ? this.snapshot.navigation.focusedRow + 1
          : 0;
    this.commit({
      all,
      favorites,
      dirty: true,
      navigation: { ...this.snapshot.navigation, focusedRow: focusIndex },
    }, true);
  }

  removeClip(id: number): void {
    const all = this.snapshot.all.filter((item) => item.id !== id);
    const favorites = this.snapshot.favorites.filter((item) => item.id !== id);
    const items = this.snapshot.mode === "favorites" ? favorites : all;
    this.commit({
      all,
      favorites,
      navigation: normalizeAfterRefresh(collapseActions(this.snapshot.navigation), items.length),
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

  async invokeAction(clip: ClipItem, action: "copy" | "favorite" | "delete"): Promise<void> {
    try {
      if (action === "copy") await selectClip(clip.id);
      else if (action === "favorite") {
        await toggleFavorite(clip.id);
        await this.refresh();
      } else {
        await deleteClip(clip.id);
        this.removeClip(clip.id);
      }
    } catch (error) {
      console.error("Clipboard action failed", error);
    }
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
    await this.invokeAction(clip, "copy");
    return true;
  }

  async loadMore(): Promise<void> {
    if (this.snapshot.loadingMore) return;
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
    this.requestGeneration += 1;
    if (this.queryTimer !== null) window.clearTimeout(this.queryTimer);
    this.queryTimer = null;
    this.commit({
      all: [],
      favorites: [],
      dirty: true,
      loadingMore: false,
      navigation: releaseNavigation(this.snapshot.navigation),
    });
  }

  markDirty(): void {
    this.commit({ dirty: true });
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
