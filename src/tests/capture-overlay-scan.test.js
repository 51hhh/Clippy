import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as i18n from "../i18n/i18n.js";
import { ScanPopover } from "../react/capture-overlay/ScanPopover.tsx";
import {
  captureScanErrorMessage,
  isCurrentCaptureScan,
} from "../react/capture-overlay/scanState.ts";

describe("capture overlay code scan", () => {
  beforeEach(() => i18n.init("en"));

  it("renders code text escaped and never creates a navigable link", () => {
    const html = renderToStaticMarkup(React.createElement(ScanPopover, {
      state: {
        status: "result",
        result: {
          limited: false,
          results: [{ format: "qr_code", text: "<script>unsafe</script>", points: [] }],
        },
      },
      left: 8,
      top: 8,
      copiedIndex: null,
      copyFailedIndex: null,
      onCopy: vi.fn(),
      onClose: vi.fn(),
    }));

    expect(html).toContain('role="dialog"');
    expect(html).toContain('aria-label="Close scan results"');
    expect(html).toContain("&lt;script&gt;unsafe&lt;/script&gt;");
    expect(html).not.toContain("<script>unsafe</script>");
    expect(html).not.toContain("<a");
  });

  it("uses stable error codes and exact generation plus selection identity", () => {
    expect(captureScanErrorMessage({ code: "busy", detail: "private" }))
      .toBe("Another local code scan is already running.");
    expect(captureScanErrorMessage({ code: "private_backend_error" }))
      .toBe("Could not scan this selection.");
    expect(isCurrentCaptureScan(4, 4, "selection-a", "selection-a")).toBe(true);
    expect(isCurrentCaptureScan(4, 3, "selection-a", "selection-a")).toBe(false);
    expect(isCurrentCaptureScan(4, 4, "selection-b", "selection-a")).toBe(false);
  });
});
