/**
 * preview/metadata-renderers.js — 时间与常见标识符元数据预览
 */

import { t } from "../../i18n/i18n.js";
import {
  formatTimestamp,
  uuidVersion,
  ipInfo,
  emailInfo,
  macInfo,
  cronDescribe,
  dateInfo,
  semverInfo,
  numberBaseInfo,
} from "./detectors.js";

export function createMetadataRenderers({ contentEl, badgeEl }) {
  /** Unix 时间戳渲染 */
  function renderTimestamp(text) {
    badgeEl.textContent = "TIMESTAMP";
    contentEl.classList.add("preview-content--encoded");
    const info = formatTimestamp(text);

    const wrapper = document.createElement("div");
    wrapper.className = "timestamp-preview";

    const rows = [
      [t("preview.tsLocal") || "Local", info.local],
      ["UTC", info.utc],
      [t("preview.tsRelative") || "Relative", info.relative],
      [t("preview.tsPrecision") || "Precision", info.precision === "ms" ? "Milliseconds" : "Seconds"],
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
      wrapper.appendChild(row);
    }

    // 原始值
    const orig = document.createElement("div");
    orig.className = "encoded-box encoded-box--muted";
    orig.textContent = text;
    wrapper.appendChild(orig);

    contentEl.appendChild(wrapper);
  }

  /** UUID 渲染 */
  function renderUuid(text) {
    const ver = uuidVersion(text);
    badgeEl.textContent = ver ? `UUID ${ver}` : "UUID";
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    if (ver) {
      const hint = document.createElement("div");
      hint.className = "encoded-hint";
      const descs = {
        v1: "Time-based (MAC address + timestamp)",
        v2: "DCE Security",
        v3: "Name-based (MD5)",
        v4: "Random",
        v5: "Name-based (SHA-1)",
        v7: "Unix Epoch time-ordered",
      };
      hint.textContent = descs[ver] || `Version ${ver}`;
      contentEl.appendChild(hint);
    }
  }

  /** IP 地址渲染 */
  function renderIpAddress(text) {
    const info = ipInfo(text);
    badgeEl.textContent = info.version;
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "ip-details";
    const items = [
      [t("preview.ipType") || "Type", info.type],
      [t("preview.ipVersion") || "Version", info.version],
    ];
    if (info.cidr) items.push(["CIDR", "Yes"]);
    for (const [label, value] of items) {
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

  /** Email 渲染 */
  function renderEmail(text) {
    const info = emailInfo(text);
    badgeEl.textContent = "EMAIL";
    contentEl.classList.add("preview-content--encoded");

    const link = document.createElement("a");
    link.href = `mailto:${text}`;
    link.className = "email-link";
    link.textContent = text;
    link.addEventListener("click", (e) => e.preventDefault());
    contentEl.appendChild(link);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.emailLocal") || "Local", info.local],
      [t("preview.emailDomain") || "Domain", info.domain],
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

  /** MAC 地址渲染 */
  function renderMac(text) {
    const info = macInfo(text);
    badgeEl.textContent = `MAC · ${info.format}`;
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = info.normalized;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.macFormat") || "Format", info.format],
      ["OUI", info.oui],
      [t("preview.macType") || "Type", info.localAdmin ? "Locally Administered" : "Universally Administered"],
      [t("preview.macCast") || "Cast", info.multicast ? "Multicast" : "Unicast"],
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

  /** Cron 表达式渲染 */
  function renderCron(text) {
    const info = cronDescribe(text);
    badgeEl.textContent = `CRON · ${info.fields} fields`;
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    const fields = text.trim().split(/\s+/);
    const labels = info.fields === 6
      ? ["Second", "Minute", "Hour", "Day", "Month", "Weekday"]
      : ["Minute", "Hour", "Day", "Month", "Weekday"];
    const table = document.createElement("div");
    table.className = "cron-fields";
    for (let i = 0; i < fields.length; i++) {
      const row = document.createElement("div");
      row.className = "timestamp-row";
      const lbl = document.createElement("span");
      lbl.className = "timestamp-label";
      lbl.textContent = labels[i];
      const val = document.createElement("span");
      val.className = "timestamp-value";
      val.textContent = fields[i];
      row.append(lbl, val);
      table.appendChild(row);
    }
    contentEl.appendChild(table);

    if (info.description) {
      const hint = document.createElement("div");
      hint.className = "encoded-hint";
      hint.textContent = info.description;
      contentEl.appendChild(hint);
    }
  }

  /** 日期字符串渲染 */
  function renderDate(text) {
    const info = dateInfo(text);
    badgeEl.textContent = "DATE";
    contentEl.classList.add("preview-content--encoded");

    const wrapper = document.createElement("div");
    wrapper.className = "timestamp-preview";

    const rows = [
      [t("preview.tsLocal") || "Local", info.local],
      ["UTC", info.utc],
      ["ISO 8601", info.iso],
      ["Unix Timestamp", String(info.timestamp)],
      [t("preview.tsRelative") || "Relative", info.relative],
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
      wrapper.appendChild(row);
    }

    const orig = document.createElement("div");
    orig.className = "encoded-box encoded-box--muted";
    orig.textContent = text;
    wrapper.appendChild(orig);

    contentEl.appendChild(wrapper);
  }

  /** 语义版本号渲染 */
  function renderSemver(text) {
    const info = semverInfo(text);
    badgeEl.textContent = "SEMVER";
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = info.normalized;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      ["Major", String(info.major)],
      ["Minor", String(info.minor)],
      ["Patch", String(info.patch)],
    ];
    if (info.preRelease) rows.push(["Pre-release", info.preRelease]);
    if (info.build) rows.push(["Build", info.build]);
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

  /** 数字进制渲染 */
  function renderNumberBase(text) {
    const info = numberBaseInfo(text);
    const baseNames = { 2: "BIN", 8: "OCT", 16: "HEX" };
    badgeEl.textContent = `NUMBER · ${baseNames[info.base] || `BASE${info.base}`}`;
    contentEl.classList.add("preview-content--encoded");

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text;
    contentEl.appendChild(box);

    const details = document.createElement("div");
    details.className = "timestamp-preview";
    const rows = [
      [t("preview.numDecimal") || "Decimal", String(info.decimal)],
      [t("preview.numHex") || "Hex", info.hex],
      [t("preview.numBin") || "Binary", info.binary],
      [t("preview.numOct") || "Octal", info.octal],
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

  /** CSS 渐变渲染 */
  function renderGradient(text) {
    badgeEl.textContent = "GRADIENT";
    contentEl.classList.add("preview-content--encoded");

    // 可视化预览色块
    const swatch = document.createElement("div");
    swatch.className = "gradient-swatch";
    swatch.style.background = text.trim();
    contentEl.appendChild(swatch);

    const box = document.createElement("pre");
    box.className = "encoded-box";
    box.textContent = text.trim();
    contentEl.appendChild(box);
  }

  return {
    renderTimestamp,
    renderUuid,
    renderIpAddress,
    renderEmail,
    renderMac,
    renderCron,
    renderDate,
    renderSemver,
    renderNumberBase,
    renderGradient,
  };
}
