import { clipboardStore } from "../react/main/clipboardStore";

export function init({ onSummonSearch, onFocusChange } = {}) {
  clipboardStore.initialize({ onSummonSearch, onFocusChange });
}

export const refresh = () => clipboardStore.refresh();
export const setQuery = (query) => clipboardStore.setQuery(query);
export const getQuery = () => clipboardStore.getSnapshot().query;
export const getFocusedClip = () => clipboardStore.getFocusedClip();
export const getLatestClip = () => clipboardStore.getLatestClip();
export const setPanelMode = (mode) => clipboardStore.setPanelMode(mode);
export const getPanelMode = () => clipboardStore.getPanelMode();
export const prependClip = (clip) => clipboardStore.prependClip(clip);
export const removeClip = (id) => clipboardStore.removeClip(id);
export const moveRow = (delta) => clipboardStore.moveRow(delta);
export const moveCol = (delta) => clipboardStore.moveCol(delta);
export const expandRowActions = () => clipboardStore.expandRowActions();
export const collapseActions = () => clipboardStore.collapseActions();
export const canExpandHere = () => clipboardStore.canExpandHere();
export const hasExpanded = () => clipboardStore.hasExpanded();
export const releaseMemory = () => clipboardStore.releaseMemory();
export const markDirty = () => clipboardStore.markDirty();
export const isDirty = () => clipboardStore.isDirty();
export const restoreRender = () => clipboardStore.restoreRender();
export const refreshLabels = () => clipboardStore.refreshLabels();
export const activateFocus = () => clipboardStore.activateFocus();
export const selectByIndex = (index) => clipboardStore.selectByIndex(index);

export const search = {
  isVisible: () => clipboardStore.isSearchVisible(),
  summon: () => clipboardStore.summonSearch(),
  dismissStage: () => clipboardStore.dismissSearchStage(),
};
