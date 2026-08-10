/**
 * preview/format-renderers.js — 数据格式与表达式预览
 */

import { t } from "../../i18n/i18n.js";
import {
  dataSizeInfo,
  regexInfo,
  coordInfo,
  mimeInfo,
  mathEval,
  httpStatusInfo,
} from "./detectors.js";

export function createFormatRenderers({ contentEl, badgeEl }) {
  /** 数据大小渲染 */
  function renderDataSize(text) {
    const info = dataSizeInfo(text);
    badgeEl.textContent = "DATA SIZE";
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text.trim();
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    for (const conv of info.conversions) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = conv;
      row.appendChild(val);
      details.appendChild(row);
    }
    contentEl.appendChild(details);
  }

  /** 正则表达式渲染 */
  function renderRegex(text) {
    const info = regexInfo(text);
    badgeEl.textContent = "REGEX";
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.regexPattern") || "Pattern", info.pattern],
      [t("preview.regexFlags") || "Flags", info.flags || "(none)"],
    ];
    if (info.flagDescs.length > 0) {
      rows.push([t("preview.regexDesc") || "Description", info.flagDescs.join(", ")]);
    }
    for (const [label, value] of rows) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const lbl = document.createElement("span");
      lbl.className = "timestamp-label";
      lbl.textContent = label;
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = value;
      row.append(lbl, val);
      details.appendChild(row);
    }
    contentEl.appendChild(details);
  }

  /** 坐标渲染 */
  function renderCoordinate(text) {
    const info = coordInfo(text);
    badgeEl.textContent = "COORDINATE";
    contentEl.classList.add("preview-content--encoded");

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.coordDecimal") || "Decimal", info.decimal],
      ["DMS", info.dms],
      [t("preview.coordLat") || "Latitude", `${info.lat >= 0 ? "N" : "S"} ${Math.abs(info.lat).toFixed(6)}°`],
      [t("preview.coordLng") || "Longitude", `${info.lng >= 0 ? "E" : "W"} ${Math.abs(info.lng).toFixed(6)}°`],
    ];
    for (const [label, value] of rows) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const lbl = document.createElement("span");
      lbl.className = "timestamp-label";
      lbl.textContent = label;
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = value;
      row.append(lbl, val);
      details.appendChild(row);
    }
    contentEl.appendChild(details);
  }

  /** MIME type 渲染 */
  function renderMimeType(text) {
    const info = mimeInfo(text);
    badgeEl.textContent = "MIME TYPE";
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.mimeType") || "Type", info.type],
      [t("preview.mimeSubtype") || "Subtype", info.subtype],
      [t("preview.mimeDesc") || "Description", info.description],
    ];
    for (const [label, value] of rows) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const lbl = document.createElement("span");
      lbl.className = "timestamp-label";
      lbl.textContent = label;
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = value;
      row.append(lbl, val);
      details.appendChild(row);
    }
    contentEl.appendChild(details);
  }

  /** 数学表达式渲染 */
  function renderMathExpr(text) {
    const info = mathEval(text);
    badgeEl.textContent = "MATH";
    contentEl.classList.add("preview-content--encoded");

    const exprBox = document.createElement("pre");
    exprBox.className = "encoded-box";
    exprBox.textContent = `${text} = ${info.result}`;
    contentEl.appendChild(exprBox);

    const resultBox = document.createElement("div");
    resultBox.className = "math-result";
    resultBox.textContent = String(info.result);
    contentEl.appendChild(resultBox);
  }

  /** HTTP 状态码渲染 */
  function renderHttpStatus(text) {
    const info = httpStatusInfo(text);
    badgeEl.textContent = `HTTP ${info.code}`;
    contentEl.classList.add("preview-content--encoded");

    const header = document.createElement("div");
    header.className = "http-status-header";
    header.textContent = `${info.code} ${info.message}`;
    contentEl.appendChild(header);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.httpCategory") || "Category", info.category],
      [t("preview.httpMessage") || "Message", info.message],
    ];
    for (const [label, value] of rows) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const lbl = document.createElement("span");
      lbl.className = "timestamp-label";
      lbl.textContent = label;
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = value;
      row.append(lbl, val);
      details.appendChild(row);
    }
    contentEl.appendChild(details);
  }

  return {
    renderDataSize,
    renderRegex,
    renderCoordinate,
    renderMimeType,
    renderMathExpr,
    renderHttpStatus,
  };
}
