/**
 * 剪贴板列表键盘导航状态机。
 *
 * 转换函数不操作 DOM，调用方负责把 nextState 同步到视图。
 */

export const ROW_BODY = -1;
export const ACTION_COUNT = 3;

export function createNavigationState() {
  return {
    focusedRow: -1,
    focusedCol: ROW_BODY,
    expandedRow: null,
    keyboardNav: false,
  };
}

export function normalizeAfterRefresh(state, itemCount) {
  const focusedRow = itemCount === 0
    ? -1
    : clamp(state.focusedRow, 0, itemCount - 1);
  return {
    ...state,
    focusedRow,
    focusedCol: ROW_BODY,
    expandedRow: null,
  };
}

export function resetForPanelChange(state) {
  return {
    ...state,
    focusedRow: 0,
    focusedCol: ROW_BODY,
    expandedRow: null,
  };
}

export function moveRowFocus(state, delta, itemCount) {
  if (itemCount === 0) return { nextState: state, summonSearch: false };
  if (delta < 0 && state.focusedRow === 0 && state.focusedCol === ROW_BODY) {
    return { nextState: state, summonSearch: true };
  }

  return {
    nextState: {
      ...state,
      focusedRow: clamp(state.focusedRow + delta, 0, itemCount - 1),
      focusedCol: ROW_BODY,
      expandedRow: null,
      keyboardNav: true,
    },
    summonSearch: false,
  };
}

export function moveColumnFocus(state, delta, itemCount, panelMode) {
  if (state.focusedCol === ROW_BODY) {
    let requestedMode = null;
    if (delta < 0 && panelMode === "all") requestedMode = "favorites";
    if (delta > 0 && panelMode === "favorites") requestedMode = "all";
    return { nextState: state, requestedMode };
  }
  if (itemCount === 0) return { nextState: state, requestedMode: null };

  // 收藏模式中操作按钮位于行左侧，因此视觉方向与按钮索引相反。
  const effectiveDelta = panelMode === "favorites" ? -delta : delta;
  const focusedCol = state.focusedCol + effectiveDelta;
  if (focusedCol < 0) {
    return {
      nextState: { ...state, focusedCol: ROW_BODY, expandedRow: null },
      requestedMode: null,
    };
  }
  if (focusedCol >= ACTION_COUNT) {
    return { nextState: state, requestedMode: null };
  }
  return {
    nextState: { ...state, focusedCol },
    requestedMode: null,
  };
}

export function expandActions(state, clipId) {
  return { ...state, expandedRow: clipId, focusedCol: 0 };
}

export function collapseActions(state) {
  return { ...state, expandedRow: null };
}

export function focusRowBody(state, focusedRow) {
  return { ...state, focusedRow, focusedCol: ROW_BODY };
}

export function focusAction(state, focusedRow, focusedCol) {
  return { ...state, focusedRow, focusedCol };
}

export function consumePointerMove(state) {
  if (!state.keyboardNav) return { nextState: state, ignore: false };
  return { nextState: { ...state, keyboardNav: false }, ignore: true };
}

/**
 * 面板关闭、列表内容被释放时的状态。
 *
 * `focusedRow` 必须是 -1（没有焦点行）而不是 0：列表已经空了，0 是个不存在的行。
 * 报成 0 会让 `prependClip` 以为用户正选着第一行，于是新条目到达时把焦点"让"到下一行——
 * 重新打开面板时焦点落在**第二行**，按 Pin 贴出的是上一条。列表重新加载时
 * `normalizeAfterRefresh` 会把 -1 收拢成 0，所以打开面板照样高亮最新那条。
 */
export function releaseNavigation(state) {
  return {
    ...state,
    focusedRow: -1,
    focusedCol: ROW_BODY,
    expandedRow: null,
  };
}

function clamp(value, lower, upper) {
  return Math.max(lower, Math.min(upper, value));
}
