import { useCallback, useEffect, useRef, useState } from "react";
import type {
  LongshotActivation,
  LongshotControllerError,
  LongshotHandle,
} from "../../js/ipc-types.ts";
import { t } from "../shared/i18n";
import { longshotControllerApi } from "./api";

type ControllerPhase = "preparing" | "ready" | "activationError" | "cancelling" | "cleanupError";

type DisplayError = Pick<LongshotControllerError, "code">;

function parseControllerError(reason: unknown): LongshotControllerError {
  if (typeof reason === "object" && reason !== null) {
    const candidate = reason as Partial<LongshotControllerError>;
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      return { code: candidate.code, message: candidate.message };
    }
  }
  return { code: "longshot_controller_internal", message: "unstructured IPC failure" };
}

function errorText(error: DisplayError | null): string {
  if (!error) return "";
  switch (error.code) {
    case "longshot_controller_cleanup_failed":
      return t("longshot.cleanupFailed");
    case "longshot_controller_superseded":
      return t("longshot.superseded");
    case "longshot_controller_busy":
      return t("longshot.busy");
    default:
      return t("longshot.failed");
  }
}

/**
 * 这个窗口必须是 ordinary 覆盖层销毁后仍在的 IPC 调用者。
 * StrictMode 会重放 effect，activationStarted 是每个原生窗口的单次屏障。
 */
export function App() {
  const activationStarted = useRef(false);
  const activationRequest = useRef<Promise<LongshotActivation> | null>(null);
  const readySubmitted = useRef(false);
  const cancelling = useRef(false);
  const [phase, setPhase] = useState<ControllerPhase>("preparing");
  const [activation, setActivation] = useState<LongshotActivation | null>(null);
  const [error, setError] = useState<DisplayError | null>(null);

  const cancel = useCallback(async (handle: LongshotHandle | null) => {
    if (cancelling.current || phase === "cleanupError") return;
    cancelling.current = true;
    setPhase("cancelling");
    setError(null);
    try {
      await longshotControllerApi.cancel(handle);
      // 成功时后端会关闭这个窗口；保留 Cancelling 以免短暂重绘出可操作的旧状态。
    } catch (reason) {
      const parsed = parseControllerError(reason);
      console.warn("长截图控制窗口取消失败", parsed.message);
      cancelling.current = false;
      setError(parsed);
      // 仅 cleanup failure 代表后端无法安全证明资源已释放，不能再尝试取消。
      setPhase(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" :
        handle ? "ready" : "activationError");
    }
  }, [phase]);

  useEffect(() => {
    if (!activationStarted.current) {
      activationStarted.current = true;
      activationRequest.current = longshotControllerApi.activate();
    }
    const request = activationRequest.current;
    if (!request) return;
    let mounted = true;
    void request
      .then((value) => {
        if (!mounted) return;
        setActivation(value);
        setPhase("ready");
      })
      .catch((reason) => {
        if (!mounted) return;
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口启动失败", parsed.message);
        setError(parsed);
        setPhase(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" : "activationError");
      });
    return () => {
      mounted = false;
    };
  }, []);

  // 必须等 activation 结果（包括不可恢复的 CleanupError）已经实际渲染后才能要求后端
  // show，避免隐藏窗口白闪，也避免把重启指引永远留在隐藏 WebView 里。
  useEffect(() => {
    if (
      (phase !== "ready" && phase !== "activationError" && phase !== "cleanupError")
      || readySubmitted.current
    ) return;
    readySubmitted.current = true;
    void longshotControllerApi.ready().catch((reason) => {
      const parsed = parseControllerError(reason);
      console.warn("长截图控制窗口显示失败", parsed.message);
      setError(parsed);
      // 只有后端明确无法证明资源已释放时才阻止再次取消。
      // 其余 show/ready 失败仍可能保有 Active handle，必须保留安全关闭路径。
      setPhase((current) => parsed.code === "longshot_controller_cleanup_failed"
        ? "cleanupError"
        : current === "activationError" ? "activationError" : "ready");
    });
  }, [phase]);

  const cancelCurrent = useCallback(() => {
    void cancel(activation?.handle ?? null);
  }, [activation, cancel]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      cancelCurrent();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cancelCurrent]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void longshotControllerApi.onCloseRequested(cancelCurrent).then((value) => {
      if (disposed) value();
      else unlisten = value;
    }).catch((reason) => console.warn("长截图控制窗口关闭监听失败", reason));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [cancelCurrent]);

  const snapshot = activation?.snapshot;
  return (
    <main className="longshot-controller" aria-live="polite">
      <header className="longshot-titlebar" data-tauri-drag-region>
        <h1 data-tauri-drag-region>{t("longshot.title")}</h1>
      </header>

      {phase === "preparing" && <p className="longshot-status">{t("longshot.preparing")}</p>}

      {phase === "ready" && snapshot && (
        <>
          <p className="longshot-status">{t("longshot.ready")}</p>
          <dl className="longshot-details">
            <div><dt>{t("longshot.frameCount")}</dt><dd>{snapshot.frameCount}</dd></div>
            <div><dt>{t("longshot.totalHeight")}</dt><dd>{snapshot.totalHeight}</dd></div>
          </dl>
          {error && <p className="longshot-error" role="alert">{errorText(error)}</p>}
          <button type="button" onClick={cancelCurrent}>{t("longshot.cancel")}</button>
        </>
      )}

      {phase === "activationError" && (
        <>
          <p className="longshot-error" role="alert">{errorText(error)}</p>
          <button type="button" onClick={cancelCurrent}>{t("longshot.close")}</button>
        </>
      )}

      {phase === "cancelling" && <p className="longshot-status">{t("longshot.cancelling")}</p>}

      {phase === "cleanupError" && (
        <p className="longshot-error" role="alert">{t("longshot.cleanupFailed")}</p>
      )}
    </main>
  );
}
