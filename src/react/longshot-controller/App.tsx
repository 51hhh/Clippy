import { useCallback, useEffect, useRef, useState } from "react";
import type {
  LongshotActivation,
  LongshotControllerError,
  LongshotHandle,
  LongshotOutputAction,
  LongshotOutputResult,
} from "../../js/ipc-types.ts";
import { t } from "../shared/i18n";
import { longshotControllerApi } from "./api";

type ControllerPhase =
  | "preparing"
  | "ready"
  | "appending"
  | "undoing"
  | "finishing"
  | "outputPending"
  | "finished"
  | "activationError"
  | "cancelling"
  | "cleanupError";

type DisplayError = Pick<LongshotControllerError, "code">;
type ErrorContext = "activation" | "append" | "finish" | LongshotOutputAction;
type PreviewStatus = "loading" | "available" | "unavailable";
type PreviewRequest = {
  identity: string;
  epoch: number;
  promise: Promise<ArrayBuffer>;
  handled: boolean;
};
/**
 * 后端 OutputPending 当前能安全接受的动作集合。Copy 没有 uncertain 结果；Save 和
 * Pin 的 uncertain 结果只会从集合中移除一个动作，绝不能重新授权它。
 */
type OutputRetryPolicy = "any" | "copySave" | "copyPin" | "copyOnly";

function allowsOutputAction(policy: OutputRetryPolicy, action: LongshotOutputAction): boolean {
  if (action === "copy") return true;
  if (policy === "any") return true;
  return policy === "copySave" ? action === "save" : policy === "copyPin" && action === "pin";
}

function withoutUncertainOutputAction(
  policy: OutputRetryPolicy,
  action: LongshotOutputAction,
): OutputRetryPolicy {
  if (action === "copy" || policy === "copyOnly") return policy;
  if (policy === "any") return action === "save" ? "copyPin" : "copySave";
  if (policy === "copySave" && action === "save") return "copyOnly";
  if (policy === "copyPin" && action === "pin") return "copyOnly";
  return policy;
}

function parseControllerError(reason: unknown): LongshotControllerError {
  if (typeof reason === "object" && reason !== null) {
    const candidate = reason as Partial<LongshotControllerError>;
    if (typeof candidate.code === "string") {
      return {
        code: candidate.code,
        message: typeof candidate.message === "string" ? candidate.message : "structured IPC failure",
      };
    }
  }
  return { code: "longshot_controller_internal", message: "unstructured IPC failure" };
}

function errorText(error: DisplayError | null, context: ErrorContext): string {
  if (!error) return "";
  switch (error.code) {
    case "longshot_controller_cleanup_failed":
      return t("longshot.cleanupFailed");
    case "longshot_controller_superseded":
      return t("longshot.superseded");
    case "longshot_controller_busy":
      return t("longshot.busy");
    case "longshot_controller_copy_failed":
      return t("longshot.copyFailed");
    case "longshot_controller_save_failed":
      return t("longshot.saveFailed");
    case "longshot_controller_save_uncertain":
      return t("longshot.saveUncertain");
    case "longshot_controller_pin_failed":
      return t("longshot.pinFailed");
    case "longshot_controller_pin_uncertain":
      return t("longshot.pinUncertain");
    default:
      if (context === "append") return t("longshot.appendFailed");
      if (context === "finish") return t("longshot.finishFailed");
      if (context === "copy") return t("longshot.copyFailed");
      if (context === "save") return t("longshot.saveFailed");
      if (context === "pin") return t("longshot.pinFailed");
      return t("longshot.failed");
  }
}

function isOutputFailure(error: DisplayError): boolean {
  return error.code === "longshot_controller_copy_failed"
    || error.code === "longshot_controller_save_failed"
    || error.code === "longshot_controller_save_uncertain"
    || error.code === "longshot_controller_pin_failed"
    || error.code === "longshot_controller_pin_uncertain";
}

/** IPC 成功响应也属于不可信边界，不能用伪成功覆盖控制窗的保守终态。 */
function isExpectedOutputResult(
  value: unknown,
  action: LongshotOutputAction,
): value is LongshotOutputResult {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<LongshotOutputResult>;
  if (candidate.action !== action) return false;
  if (action === "copy") return candidate.path === null && candidate.pinLabel === null;
  if (action === "save") {
    return typeof candidate.path === "string" && candidate.path.length > 0 && candidate.pinLabel === null;
  }
  return candidate.path === null
    && typeof candidate.pinLabel === "string"
    && candidate.pinLabel.length > 0;
}

/**
 * 这个窗口必须是 ordinary 覆盖层销毁后仍在的 IPC 调用者。
 * StrictMode 会重放 effect，activationStarted 是每个原生窗口的单次屏障。
 */
export function App() {
  const activationStarted = useRef(false);
  const activationRequest = useRef<Promise<LongshotActivation> | null>(null);
  const readyRequest = useRef<Promise<void> | null>(null);
  const cancelling = useRef(false);
  const mounted = useRef(false);
  const lifecycleEpoch = useRef(0);
  const appendEpoch = useRef(0);
  const appendInFlight = useRef(false);
  const finishEpoch = useRef(0);
  const finishInFlight = useRef(false);
  const outputRetryPolicyRef = useRef<OutputRetryPolicy>("any");
  const phaseRef = useRef<ControllerPhase>("preparing");
  const previewEpoch = useRef(0);
  const previewRequest = useRef<PreviewRequest | null>(null);
  const previewUrlRef = useRef<string | null>(null);
  const [phase, setPhase] = useState<ControllerPhase>("preparing");
  const [activation, setActivation] = useState<LongshotActivation | null>(null);
  const [error, setError] = useState<DisplayError | null>(null);
  const [errorContext, setErrorContext] = useState<ErrorContext>("activation");
  const [finishingAction, setFinishingAction] = useState<LongshotOutputAction | null>(null);
  const [finishedResult, setFinishedResult] = useState<LongshotOutputResult | null>(null);
  const [outputRetryPolicy, setOutputRetryPolicy] = useState<OutputRetryPolicy>("any");
  const [readyComplete, setReadyComplete] = useState(false);
  const [previewStatus, setPreviewStatus] = useState<PreviewStatus>("loading");
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  const transition = (next: ControllerPhase) => {
    phaseRef.current = next;
    setPhase(next);
  };

  const invalidateAppendAttempt = () => {
    appendEpoch.current += 1;
    appendInFlight.current = false;
  };

  const invalidateFinishAttempt = () => {
    finishEpoch.current += 1;
    finishInFlight.current = false;
  };

  const invalidatePreviewAttempt = () => {
    previewEpoch.current += 1;
  };

  const releasePreviewUrl = () => {
    const owned = previewUrlRef.current;
    previewUrlRef.current = null;
    if (owned !== null) URL.revokeObjectURL(owned);
    if (mounted.current) setPreviewUrl(null);
  };

  const setPendingRetryPolicy = (policy: OutputRetryPolicy) => {
    outputRetryPolicyRef.current = policy;
    setOutputRetryPolicy(policy);
  };

  const cancel = useCallback(async (handle: LongshotHandle | null) => {
    if (
      cancelling.current
      || phaseRef.current === "cleanupError"
      || phaseRef.current === "finished"
    ) return;
    const phaseBeforeCancel = phaseRef.current;
    // 取消是所有本地异步 promise 的线性化边界：它之后的任何完成都不得复活 UI。
    invalidateAppendAttempt();
    invalidateFinishAttempt();
    invalidatePreviewAttempt();
    setFinishingAction(null);
    cancelling.current = true;
    transition("cancelling");
    setError(null);
    try {
      await longshotControllerApi.cancel(handle);
      releasePreviewUrl();
      // 成功时后端会关闭这个窗口；保留 Cancelling 以免短暂重绘出可操作的旧状态。
    } catch (reason) {
      if (!mounted.current) return;
      const parsed = parseControllerError(reason);
      console.warn("长截图控制窗口取消失败", parsed.message);
      cancelling.current = false;
      setError(parsed);
      setErrorContext("activation");
      // 仅 cleanup failure 代表后端无法安全证明资源已释放，不能再尝试取消。
      transition(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" :
        handle ? phaseBeforeCancel === "outputPending" ? "outputPending" : "ready" : "activationError");
      if (parsed.code === "longshot_controller_cleanup_failed") releasePreviewUrl();
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    lifecycleEpoch.current += 1;
    if (!activationStarted.current) {
      activationStarted.current = true;
      activationRequest.current = longshotControllerApi.activate();
    }
    const request = activationRequest.current;
    if (!request) return;
    let effectMounted = true;
    void request
      .then((value) => {
        if (!effectMounted) return;
        setActivation(value);
        transition("ready");
      })
      .catch((reason) => {
        if (!effectMounted) return;
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口启动失败", parsed.message);
        setError(parsed);
        setErrorContext("activation");
        transition(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" : "activationError");
      });
    return () => {
      effectMounted = false;
      mounted.current = false;
      const cleanupEpoch = ++lifecycleEpoch.current;
      queueMicrotask(() => {
        // React StrictMode 会同步重挂 effect；只在没有重挂时执行真实卸载清理。
        if (mounted.current || lifecycleEpoch.current !== cleanupEpoch) return;
        invalidateAppendAttempt();
        invalidateFinishAttempt();
        invalidatePreviewAttempt();
        releasePreviewUrl();
      });
    };
  }, []);

  // 必须等 activation 结果（包括不可恢复的 CleanupError）已经实际渲染后才能要求后端
  // show，避免隐藏窗口白闪，也避免把重启指引永远留在隐藏 WebView 里。
  useEffect(() => {
    let disposed = false;
    if (
      (phase !== "ready" && phase !== "activationError" && phase !== "cleanupError")
    ) return () => {
      disposed = true;
    };
    if (!readyRequest.current) readyRequest.current = longshotControllerApi.ready();
    void readyRequest.current
      .then(() => {
        if (!disposed) setReadyComplete(true);
      })
      .catch((reason) => {
        if (disposed) return;
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口显示失败", parsed.message);
        setError(parsed);
        setErrorContext("activation");
        // 只有后端明确无法证明资源已释放时才阻止再次取消。
        // 其余 show/ready 失败仍可能保有 Active handle，必须保留安全关闭路径。
        transition(parsed.code === "longshot_controller_cleanup_failed"
          ? "cleanupError"
          : phaseRef.current === "activationError" ? "activationError" : "ready");
        if (parsed.code === "longshot_controller_cleanup_failed") releasePreviewUrl();
      });
    return () => {
      disposed = true;
    };
  }, [phase]);

  const snapshot = activation?.snapshot;
  const previewIdentity = activation && snapshot
    ? `${activation.handle.sessionId}:${activation.handle.generation}:${snapshot.frameCount}:${snapshot.width}:${snapshot.totalHeight}`
    : null;

  useEffect(() => {
    if (!readyComplete || !activation || !previewIdentity || phase !== "ready") return;
    let request = previewRequest.current;
    const canReuseRequest = request
      && request.identity === previewIdentity
      && (request.epoch === previewEpoch.current || request.handled);
    if (!canReuseRequest) {
      const epoch = ++previewEpoch.current;
      request = {
        identity: previewIdentity,
        epoch,
        promise: longshotControllerApi.preview(activation.handle),
        handled: false,
      };
      previewRequest.current = request;
      if (previewUrlRef.current === null) setPreviewStatus("loading");
    }
    if (!request) return;
    let disposed = false;
    const observedRequest = request;
    void observedRequest.promise
      .then((bytes) => {
        // 所有 stale 判定都在创建 object URL 前完成，避免制造无人拥有的 URL。
        if (
          disposed
          || !mounted.current
          || previewRequest.current !== observedRequest
          || observedRequest.epoch !== previewEpoch.current
          || phaseRef.current !== "ready"
          || observedRequest.handled
        ) return;
        observedRequest.handled = true;
        let nextUrl: string;
        try {
          nextUrl = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
        } catch {
          if (previewUrlRef.current === null) setPreviewStatus("unavailable");
          return;
        }
        const previousUrl = previewUrlRef.current;
        previewUrlRef.current = nextUrl;
        if (previousUrl !== null) URL.revokeObjectURL(previousUrl);
        setPreviewUrl(nextUrl);
        setPreviewStatus("available");
      })
      .catch(() => {
        if (
          disposed
          || !mounted.current
          || previewRequest.current !== observedRequest
          || observedRequest.epoch !== previewEpoch.current
          || phaseRef.current !== "ready"
          || observedRequest.handled
        ) return;
        observedRequest.handled = true;
        if (previewUrlRef.current === null) setPreviewStatus("unavailable");
      });
    return () => {
      disposed = true;
    };
  }, [activation, phase, previewIdentity, readyComplete]);

  const appendCurrent = useCallback(() => {
    const currentActivation = activation;
    if (
      !currentActivation
      || phaseRef.current !== "ready"
      || cancelling.current
      || appendInFlight.current
    ) return;

    appendInFlight.current = true;
    const attempt = ++appendEpoch.current;
    invalidatePreviewAttempt();
    const previewBarrier = previewRequest.current?.promise;
    setError(null);
    setErrorContext("append");
    transition("appending");

    void (async () => {
      if (previewBarrier) {
        try {
          await previewBarrier;
        } catch {
          // Preview 失败只用于解除互斥屏障，不进入捕获错误状态机。
        }
      }
      if (
        !mounted.current
        || attempt !== appendEpoch.current
        || !appendInFlight.current
        || cancelling.current
        || phaseRef.current !== "appending"
      ) return;
      try {
        const nextSnapshot = await longshotControllerApi.append(currentActivation.handle);
        if (
          !mounted.current
          || attempt !== appendEpoch.current
          || cancelling.current
          || phaseRef.current !== "appending"
        ) return;
        appendInFlight.current = false;
        setActivation((current) => current ? { ...current, snapshot: nextSnapshot } : current);
        setError(null);
        transition("ready");
      } catch (reason) {
        if (
          !mounted.current
          || attempt !== appendEpoch.current
          || cancelling.current
          || phaseRef.current !== "appending"
        ) return;
        appendInFlight.current = false;
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口追加失败", parsed.message);
        setError(parsed);
        setErrorContext("append");
        transition(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" : "ready");
        if (parsed.code === "longshot_controller_cleanup_failed") releasePreviewUrl();
      }
    })();
  }, [activation]);

  const undoCurrent = useCallback(() => {
    const currentActivation = activation;
    if (
      !currentActivation
      || currentActivation.snapshot.frameCount <= 1
      || phaseRef.current !== "ready"
      || cancelling.current
      || appendInFlight.current
    ) return;

    appendInFlight.current = true;
    const attempt = ++appendEpoch.current;
    invalidatePreviewAttempt();
    const previewBarrier = previewRequest.current?.promise;
    setError(null);
    setErrorContext("append");
    transition("undoing");

    void (async () => {
      if (previewBarrier) {
        try {
          await previewBarrier;
        } catch {
          // Preview 失败只用于解除互斥屏障。
        }
      }
      if (
        !mounted.current
        || attempt !== appendEpoch.current
        || !appendInFlight.current
        || cancelling.current
        || phaseRef.current !== "undoing"
      ) return;
      try {
        const nextSnapshot = await longshotControllerApi.undo(currentActivation.handle);
        if (
          !mounted.current
          || attempt !== appendEpoch.current
          || cancelling.current
          || phaseRef.current !== "undoing"
        ) return;
        appendInFlight.current = false;
        setActivation((current) => current ? { ...current, snapshot: nextSnapshot } : current);
        transition("ready");
      } catch (reason) {
        if (
          !mounted.current
          || attempt !== appendEpoch.current
          || cancelling.current
          || phaseRef.current !== "undoing"
        ) return;
        appendInFlight.current = false;
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口撤销失败", parsed.message);
        setError(parsed);
        setErrorContext("append");
        transition(parsed.code === "longshot_controller_cleanup_failed" ? "cleanupError" : "ready");
        if (parsed.code === "longshot_controller_cleanup_failed") releasePreviewUrl();
      }
    })();
  }, [activation]);

  const finishCurrent = useCallback((action: LongshotOutputAction) => {
    const currentActivation = activation;
    const retryingOutput = phaseRef.current === "outputPending";
    if (
      !currentActivation
      || (phaseRef.current !== "ready" && !retryingOutput)
      || cancelling.current
      || finishInFlight.current
      || (retryingOutput && !allowsOutputAction(outputRetryPolicyRef.current, action))
    ) return;

    finishInFlight.current = true;
    const attempt = ++finishEpoch.current;
    invalidatePreviewAttempt();
    const previewBarrier = previewRequest.current?.promise;
    setError(null);
    setErrorContext(retryingOutput ? action : "finish");
    setFinishingAction(action);
    setFinishedResult(null);
    transition("finishing");

    void (async () => {
      if (previewBarrier) {
        try {
          await previewBarrier;
        } catch {
          // Preview 降级不改变输出动作的既有错误与重试策略。
        }
      }
      if (
        !mounted.current
        || attempt !== finishEpoch.current
        || !finishInFlight.current
        || cancelling.current
        || phaseRef.current !== "finishing"
      ) return;
      try {
        const result = await longshotControllerApi.finish(currentActivation.handle, action);
        if (
          !mounted.current
          || attempt !== finishEpoch.current
          || cancelling.current
          || phaseRef.current !== "finishing"
        ) return;
        finishInFlight.current = false;
        setFinishingAction(null);
        if (!isExpectedOutputResult(result, action)) {
          // 输出副作用是否已完成无法由异常响应证明，按 cleanup 终态收口而非渲染伪成功。
          console.warn("长截图控制窗口完成响应无效");
          setError({ code: "longshot_controller_cleanup_failed" });
          setErrorContext("finish");
          transition("cleanupError");
          releasePreviewUrl();
          return;
        }
        // 成功后由后端清空 exact owner 并关闭原生窗口；终态禁止旧回调重新开放操作。
        setFinishedResult(result);
        transition("finished");
        releasePreviewUrl();
      } catch (reason) {
        if (
          !mounted.current
          || attempt !== finishEpoch.current
          || cancelling.current
          || phaseRef.current !== "finishing"
        ) return;
        finishInFlight.current = false;
        setFinishingAction(null);
        const parsed = parseControllerError(reason);
        console.warn("长截图控制窗口完成输出失败", parsed.message);
        setError(parsed);
        if (parsed.code === "longshot_controller_cleanup_failed") {
          setErrorContext("finish");
          transition("cleanupError");
          releasePreviewUrl();
        } else if (isOutputFailure(parsed) || retryingOutput) {
          // OutputPending 只能继续输出或丢弃，不能错误地重新开放 Append。Save/Pin 的
          // uncertain 结果只会收紧集合；后续业务失败也不能升级此前移除的动作。
          setPendingRetryPolicy(
            parsed.code === "longshot_controller_save_uncertain"
              ? withoutUncertainOutputAction(outputRetryPolicyRef.current, "save")
              : parsed.code === "longshot_controller_pin_uncertain"
                ? withoutUncertainOutputAction(outputRetryPolicyRef.current, "pin")
                : retryingOutput ? outputRetryPolicyRef.current : "any",
          );
          setErrorContext(action);
          transition("outputPending");
        } else {
          // 普通编码/领域错误由后端恢复 exact Active；保留旧快照以便继续 Append 或 Copy。
          setErrorContext("finish");
          transition("ready");
        }
      }
    })();
  }, [activation]);

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

  const isBusy = phase === "appending"
    || phase === "undoing"
    || phase === "finishing"
    || phase === "cancelling";
  return (
    <main className="longshot-controller" aria-live="polite" aria-busy={isBusy}>
      <header className="longshot-titlebar" data-tauri-drag-region>
        <h1 data-tauri-drag-region>{t("longshot.title")}</h1>
      </header>

      {phase === "preparing" && <p className="longshot-status">{t("longshot.preparing")}</p>}

      {(
        phase === "ready"
        || phase === "appending"
        || phase === "undoing"
        || phase === "finishing"
        || phase === "outputPending"
        || phase === "finished"
      ) && snapshot && (
        <>
          <p className="longshot-status">
            {phase === "appending" && t("longshot.appending")}
            {phase === "undoing" && t("longshot.undoing")}
            {phase === "finishing" && finishingAction === "copy" && t("longshot.finishingCopy")}
            {phase === "finishing" && finishingAction === "save" && t("longshot.finishingSave")}
            {phase === "finishing" && finishingAction === "pin" && t("longshot.finishingPin")}
            {phase === "finished" && finishedResult?.action === "copy" && t("longshot.copied")}
            {phase === "finished" && finishedResult?.action === "save" && t("longshot.saved", {
              path: finishedResult.path,
            })}
            {phase === "finished" && finishedResult?.action === "pin" && t("longshot.pinned")}
            {phase === "ready" && t("longshot.ready")}
            {phase === "outputPending" && t("longshot.outputPending")}
          </p>
          <dl className="longshot-details">
            <div><dt>{t("longshot.frameCount")}</dt><dd>{snapshot.frameCount}</dd></div>
            <div><dt>{t("longshot.totalWidth")}</dt><dd>{snapshot.width}</dd></div>
            <div><dt>{t("longshot.totalHeight")}</dt><dd>{snapshot.totalHeight}</dd></div>
          </dl>
          {phase !== "finished" && (
            <section
              className="longshot-preview"
              role="region"
              aria-label={t("longshot.previewRegion")}
              data-testid="longshot-preview"
            >
              {previewUrl ? (
                <img src={previewUrl} alt={t("longshot.previewAlt")} />
              ) : (
                <p className="longshot-preview-status">
                  {previewStatus === "loading"
                    ? t("longshot.previewLoading")
                    : t("longshot.previewUnavailable")}
                </p>
              )}
            </section>
          )}
          {phase === "ready" && error && (
            <p className="longshot-error" role="alert">{errorText(error, errorContext)}</p>
          )}
          {phase === "outputPending" && error && (
            <p className="longshot-error" role="alert">{errorText(error, errorContext)}</p>
          )}
          {phase !== "finished" && (
            <div className="longshot-actions">
              {(phase === "ready" || phase === "appending" || phase === "undoing" || phase === "finishing") && (
                <>
                  <button
                    type="button"
                    data-testid="longshot-append"
                    onClick={appendCurrent}
                    disabled={phase !== "ready"}
                  >
                    {t("longshot.append")}
                  </button>
                  <button
                    type="button"
                    data-testid="longshot-undo"
                    onClick={undoCurrent}
                    disabled={phase !== "ready" || snapshot.frameCount <= 1}
                  >
                    {t("longshot.undo")}
                  </button>
                  <button
                    type="button"
                    data-testid="longshot-copy"
                    onClick={() => finishCurrent("copy")}
                    disabled={phase !== "ready"}
                  >
                    {t("longshot.copy")}
                  </button>
                  <button
                    type="button"
                    data-testid="longshot-save"
                    onClick={() => finishCurrent("save")}
                    disabled={phase !== "ready"}
                  >
                    {t("longshot.save")}
                  </button>
                  <button
                    type="button"
                    data-testid="longshot-pin"
                    onClick={() => finishCurrent("pin")}
                    disabled={phase !== "ready"}
                  >
                    {t("longshot.pin")}
                  </button>
                </>
              )}
              {phase === "outputPending" && (
                <>
                  <button
                    type="button"
                    data-testid="longshot-retry-copy"
                    onClick={() => finishCurrent("copy")}
                  >
                    {t("longshot.retryCopy")}
                  </button>
                  {allowsOutputAction(outputRetryPolicy, "save") && (
                    <button
                      type="button"
                      data-testid="longshot-retry-save"
                      onClick={() => finishCurrent("save")}
                    >
                      {t("longshot.retrySave")}
                    </button>
                  )}
                  {allowsOutputAction(outputRetryPolicy, "pin") && (
                    <button
                      type="button"
                      data-testid="longshot-retry-pin"
                      onClick={() => finishCurrent("pin")}
                    >
                      {t("longshot.retryPin")}
                    </button>
                  )}
                  <button type="button" data-testid="longshot-discard" onClick={cancelCurrent}>
                    {t("longshot.discard")}
                  </button>
                </>
              )}
              {(phase === "ready" || phase === "appending" || phase === "undoing" || phase === "finishing") && (
                <button type="button" data-testid="longshot-cancel" onClick={cancelCurrent}>
                  {t("longshot.cancel")}
                </button>
              )}
            </div>
          )}
        </>
      )}

      {phase === "activationError" && (
        <>
          <p className="longshot-error" role="alert">{errorText(error, errorContext)}</p>
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
