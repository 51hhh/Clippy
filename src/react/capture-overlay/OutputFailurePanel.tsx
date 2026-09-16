import type { CaptureAction, CaptureOutputError } from "../../js/ipc-types.ts";
import { t } from "../shared/i18n";

/** 失败面板只操作后端已保留的产物，不能把当前画布重新提交成另一次输出。 */
export function OutputFailurePanel(props: {
  failure: CaptureOutputError;
  message: string | null;
  busy: boolean;
  onRetry: (action: CaptureAction) => void;
  onDiscard: () => void;
}) {
  return <div className="overlay-output-failure" role="dialog" aria-modal="true"
    aria-label={t("capture.outputFailed")} onPointerDown={(event) => event.stopPropagation()}>
    <p>{t("capture.outputFailed")}</p>
    <p>{props.message || t("capture.outputWorking")}</p>
    <p>{t(props.failure.retryActions.length === 1 ? "capture.outputUncertain" : "capture.outputRetained")}</p>
    <div className="overlay-output-actions">
      {props.failure.retryActions.map((action) => <button type="button" key={action}
        disabled={props.busy} autoFocus={action === "copy"}
        onClick={() => props.onRetry(action)}>{t(`capture.${action}`)}</button>)}
      <button type="button" disabled={props.busy} onClick={props.onDiscard}>{t("capture.discardOutput")}</button>
    </div>
  </div>;
}
