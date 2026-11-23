import { useEffect, useRef, useState } from "react";
import type { ViewerHandle } from "../../js/ipc-types";
import type { ViewerServices } from "./api";

type Action = "fullscreen" | "minimize" | "drag";
type State = { fullscreen: boolean | null; pending: Action | null; failed: boolean };
const initial: State = { fullscreen: null, pending: null, failed: false };

/** 原生状态是权威；事件合并查询，旧查询/旧窗口的成功、错误均不得倒写新状态。 */
export function useViewerWindow(handle: ViewerHandle, services: ViewerServices, blocked: () => boolean) {
  const [state, setState] = useState<State>(initial);
  const blockedRef = useRef(blocked); blockedRef.current = blocked;
  const commands = useRef<{ run: (action: Action, exitOnly?: boolean, whenWindowed?: () => void) => void } | null>(null);
  useEffect(() => {
    let current = true, revision = 0, again = false, validRead = false;
    let local = initial, read: Promise<void> | null = null;
    let failure: "query" | "action" | "listener" | null = null;
    let unlisten: (() => void) | undefined;
    const publish = (change: Partial<State>) => { local = { ...local, ...change }; if (current) setState(local); };
    publish(initial);
    function refresh(): Promise<void> {
      if (!current) return Promise.resolve();
      if (read) { again = true; return read; }
      read = Promise.resolve().then(async () => {
        try {
          do {
            again = false; validRead = false; const expected = ++revision;
            try {
              const fullscreen = await services.getFullscreen(handle);
              if (current && expected === revision) {
                validRead = true;
                if (failure === "query") { failure = null; publish({ fullscreen, failed: false }); }
                else publish({ fullscreen });
              }
            } catch {
              if (current && expected === revision) { failure ||= "query"; publish({ failed: true }); }
            }
          } while (current && again);
        } finally {
          // 在查询Promise变为已完成之前释放门闩，避免同一微任务队列中新请求丢失。
          read = null;
        }
      });
      return read;
    }
    const run = (action: Action, exitOnly = false, whenWindowed?: () => void) => {
      if (!current || blockedRef.current() || local.pending) return;
      failure = null;
      publish({ pending: action, failed: false });
      // 已在途的getter不能在setFullscreen之后发布旧值；随后合并一次权威查询。
      revision++;
      void (async () => {
        try {
          if (action === "fullscreen") {
            // Esc可能紧随外部WM切换：必须新查询成功后才允许关闭工具/文档。
            if (exitOnly || local.fullscreen === null) {
              await refresh();
              if (!validRead) return;
            }
            if (!current || blockedRef.current() || local.fullscreen === null) return;
            if (exitOnly && !local.fullscreen) { whenWindowed?.(); return; }
            if (!exitOnly || local.fullscreen) await services.setFullscreen(handle, exitOnly ? false : !local.fullscreen);
            if (current) { revision++; await refresh(); }
          } else if (action === "minimize") await services.minimize(handle);
          else await services.startDrag(handle);
        } catch {
          if (current) {
            failure = "action"; publish({ failed: true });
            // setter返回错误时原生状态也可能已改变，读回真实值，保留失败提示供重试。
            if (action === "fullscreen") { revision++; await refresh(); }
          }
        }
        finally { if (current) publish({ pending: null }); }
      })();
    };
    commands.current = { run };
    void services.onWindowChanged(() => { void refresh(); }).then(dispose => {
      if (!current) dispose(); else { unlisten = dispose; void refresh(); }
    }).catch(() => { if (current) { failure = "listener"; publish({ failed: true }); } });
    void refresh();
    return () => { current = false; revision++; commands.current = null; unlisten?.(); };
  }, [handle, services]);
  return { ...state,
    toggleFullscreen: () => commands.current?.run("fullscreen"),
    minimize: () => commands.current?.run("minimize"),
    startDrag: () => commands.current?.run("drag"),
    escape: (whenWindowed: () => void) => commands.current?.run("fullscreen", true, whenWindowed),
  };
}
