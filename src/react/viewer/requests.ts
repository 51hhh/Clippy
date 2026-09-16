import type { ViewerHandle, ViewerReply, ViewerRequest } from "../../js/ipc-types";

/** 一个不可变文档拥有独立请求计数；同通道旧完成与卸载后完成都不能发布。 */
export class ViewerRequests {
  private next = 0;
  private current = new Map<string, number>();
  private active = true;
  constructor(readonly handle: ViewerHandle) {}
  activate() { this.active = true; }
  dispose() { this.active = false; this.current.clear(); }
  begin(channel: string): ViewerRequest {
    const request = { ...this.handle, requestId: ++this.next };
    this.current.set(channel, request.requestId);
    return request;
  }
  accepts(channel: string, request: ViewerRequest, reply?: ViewerReply<unknown>): boolean {
    return this.active && this.current.get(channel) === request.requestId
      && (!reply || (reply.sessionId === request.sessionId && reply.snapshotId === request.snapshotId && reply.requestId === request.requestId));
  }
}
