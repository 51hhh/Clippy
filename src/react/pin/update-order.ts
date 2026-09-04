import type { PinPayload, PinState, PinUpdate } from "./types";

/** 只有没有更新中的本地变更时，才允许 IPC 响应回写 Pin 状态。 */
export function shouldApplyPinUpdateResponse(
  requestGeneration: number,
  currentGeneration: number,
  pendingUpdate: PinUpdate,
): boolean {
  return requestGeneration === currentGeneration && Object.keys(pendingUpdate).length === 0;
}

/**
 * 把 `update_pin` 的轻量应答合并进手里的完整 payload。
 *
 * 应答故意不带图片/`text`（缩放时每帧一次，重传一遍图纯属浪费），所以内容字段
 * 只能来自本地那份。这样也顺带保证 `pin-frame` URL 不会因为一次缩放被重建。
 */
export function mergePinState(
  current: PinPayload | null,
  state: PinState,
): PinPayload | null {
  return current ? { ...current, ...state } : null;
}
