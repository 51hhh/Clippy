import { Copy, LoaderCircle, QrCode, TriangleAlert, X } from "lucide-react";
import { useEffect, useRef } from "react";
import type { ImageCodeScanResponse } from "../../js/api/validators.ts";
import { t } from "../shared/i18n";

export type CaptureScanState =
  | { status: "loading" }
  | { status: "result"; result: ImageCodeScanResponse }
  | { status: "error"; message: string };

type Props = {
  state: CaptureScanState;
  left: number;
  top: number;
  copiedIndex: number | null;
  copyFailedIndex: number | null;
  onCopy: (index: number) => void;
  onClose: () => void;
};

/** 截图选区扫码结果只展示文本；不把不可信内容变成链接或自动导航。 */
export function ScanPopover(props: Props) {
  const panel = useRef<HTMLElement>(null);
  useEffect(() => panel.current?.focus(), []);

  return (
    <section
      ref={panel}
      className="scan-popover"
      role="dialog"
      aria-modal="false"
      aria-label={t("capture.scan.title")}
      tabIndex={-1}
      style={{ left: props.left, top: props.top }}
      onPointerDown={(event) => event.stopPropagation()}
      onPointerMove={(event) => event.stopPropagation()}
      onPointerUp={(event) => event.stopPropagation()}
      onPointerCancel={(event) => event.stopPropagation()}
    >
      <header className="scan-header">
        <h2><QrCode size={15} />{t("capture.scan.title")}</h2>
        <button type="button" aria-label={t("capture.scan.close")} onClick={props.onClose}>
          <X size={15} />
        </button>
      </header>
      {props.state.status === "loading" && (
        <div className="scan-progress" role="status">
          <LoaderCircle className="scan-spinner" size={18} />
          <div><p>{t("codeScan.scanning")}</p><span>{t("capture.scan.localPrivacy")}</span></div>
        </div>
      )}
      {props.state.status === "error" && (
        <div className="scan-failure" role="alert">
          <TriangleAlert size={18} /><p>{props.state.message}</p>
        </div>
      )}
      {props.state.status === "result" && (
        <div className="scan-results">
          {props.state.result.limited && <p className="scan-notice">{t("codeScan.limited")}</p>}
          {props.state.result.results.length === 0 && <p className="scan-empty">{t("codeScan.empty")}</p>}
          {props.state.result.results.map((code, index) => (
            <section className="scan-result" key={`${code.format}-${index}`}>
              <header>
                <strong>{code.format === "qr_code" ? "QR Code" : "Code 39"}</strong>
                <button
                  type="button"
                  aria-label={t("capture.scan.copyCode", { index: index + 1 })}
                  onClick={() => props.onCopy(index)}
                >
                  <Copy size={13} />
                  {props.copiedIndex === index
                    ? t("codeScan.copied")
                    : props.copyFailedIndex === index
                      ? t("codeScan.copyFailed")
                      : t("capture.scan.copy")}
                </button>
              </header>
              <pre tabIndex={0}>{code.text}</pre>
            </section>
          ))}
        </div>
      )}
    </section>
  );
}
