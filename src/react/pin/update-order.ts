import type { PinUpdate } from "./types";

/** 只有没有更新中的本地变更时，才允许 IPC 响应回写 Pin 状态。 */
export function shouldApplyPinUpdateResponse(
  requestGeneration: number,
  currentGeneration: number,
  pendingUpdate: PinUpdate,
): boolean {
  return requestGeneration === currentGeneration && Object.keys(pendingUpdate).length === 0;
}
