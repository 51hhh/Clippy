import { describe, expect, it } from "vitest";

import {
  QA_PROFILES,
  casesForProfile,
  createQaTemplate,
  formatQaReport,
  verifyQaRecord,
} from "../../scripts/manual-qa.mjs";

const SHA = "a".repeat(40);

function completedRecord(profileId) {
  const record = createQaTemplate({ profileId, commit: SHA, appVersion: "1.2.3" });
  record.testedAt = "2026-09-02T03:00:00Z";
  record.environment.osVersion = "test-version";
  if (!record.environment.architecture) record.environment.architecture = "x86_64";
  for (const [index, result] of record.results.entries()) {
    const contract = casesForProfile(profileId)[index];
    result.status = contract.acceptedStatuses[0];
    result.observedReasonCode = contract.acceptedReasonCodes?.[0] ?? null;
    result.observation = "已按步骤验证";
    result.evidence = [`evidence/${result.id}.txt`];
  }
  return record;
}

describe("真机 QA 合同", () => {
  it("覆盖 PRD 的九个目标环境", () => {
    expect(Object.keys(QA_PROFILES)).toEqual([
      "linux-gnome-x11",
      "linux-gnome-wayland",
      "linux-gnome-wayland-ubuntu24",
      "linux-kde-wayland",
      "linux-wlroots-wayland",
      "windows-10-x64",
      "windows-11-x64",
      "macos-intel",
      "macos-apple-silicon",
    ]);
  });

  it("Ubuntu 24 单独验证 Portal 版本与截图 fallback", () => {
    const ids = casesForProfile("linux-gnome-wayland-ubuntu24").map((testCase) => testCase.id);

    expect(ids).toContain("portal_capability_versions");
    expect(ids).toContain("wayland_capture_fallbacks");
  });

  it.each(["macos-intel", "macos-apple-silicon"])(
    "%s 的签名合同与当前 Ad-Hoc 发布策略一致",
    (profileId) => {
      const signingCase = casesForProfile(profileId).find(
        (testCase) => testCase.id === "adhoc_bundle_boundary",
      );

      expect(signingCase).toEqual({
        id: "adhoc_bundle_boundary",
        title: "Ad-Hoc 签名、目标架构、未公证边界与首次打开恢复",
        acceptedStatuses: ["pass"],
      });
      expect(
        casesForProfile(profileId).some(
          (testCase) => testCase.id === "signed_notarized_bundle",
        ),
      ).toBe(false);
    },
  );

  it("公共合同覆盖完整产品能力而不只覆盖平台后端", () => {
    const ids = new Set(casesForProfile("linux-gnome-x11").map((testCase) => testCase.id));

    for (const id of [
      "clipboard_monitoring",
      "history_search_favorites",
      "keyboard_navigation",
      "codec_tools",
      "rich_preview_safety",
      "translation_roundtrip",
      "translation_privacy_tts",
      "settings_theme_i18n",
      "tray_single_instance",
      "autostart_roundtrip",
      "tmux_platform_behavior",
      "diagnostics_privacy",
    ]) {
      expect(ids.has(id), `${id} 应进入每个平台的公共合同`).toBe(true);
    }
  });

  it.each(Object.keys(QA_PROFILES))("%s 的完整证据可以通过", (profileId) => {
    const verification = verifyQaRecord(completedRecord(profileId));
    expect(verification.errors).toEqual([]);
    expect(verification.passed).toBe(true);
  });

  it("模板中的 not_run 不能冒充通过", () => {
    const record = createQaTemplate({
      profileId: "windows-11-x64",
      commit: SHA,
      appVersion: "1.2.3",
    });
    record.testedAt = "2026-09-02T03:00:00Z";
    record.environment.osVersion = "Windows 11";

    const verification = verifyQaRecord(record);
    expect(verification.passed).toBe(false);
    expect(verification.errors).toContain("clipboard_text 状态 not_run 不满足 pass");
  });

  it("模板直接写明每项允许状态和可接受 reason code", () => {
    const record = createQaTemplate({
      profileId: "windows-11-x64",
      commit: SHA,
      appVersion: "1.2.3",
    });
    const highIntegrity = record.results.find((result) => result.id === "auto_paste_high_integrity");

    expect(highIntegrity.acceptedStatuses).toEqual(["expected_degraded"]);
    expect(highIntegrity.acceptedReasonCodes).toEqual(["windows_integrity_boundary"]);
  });

  it("缺场景、重复场景和额外场景全部失败", () => {
    const record = completedRecord("linux-gnome-x11");
    record.results.splice(0, 1);
    record.results.push({ ...record.results[0] }, { id: "invented", status: "pass" });

    const verification = verifyQaRecord(record);
    expect(verification.passed).toBe(false);
    expect(verification.errors).toContain("clipboard_text 必须且只能出现一次");
    expect(verification.errors).toContain("clipboard_html 必须且只能出现一次");
    expect(verification.errors).toContain("存在未知场景: invented");
  });

  it("安全降级必须记录精确 reason code", () => {
    const record = completedRecord("windows-11-x64");
    const highIntegrity = record.results.find((result) => result.id === "auto_paste_high_integrity");
    highIntegrity.observedReasonCode = "backend_unavailable";

    const verification = verifyQaRecord(record);
    expect(verification.passed).toBe(false);
    expect(verification.errors).toContain(
      "auto_paste_high_integrity 必须观测 reason code windows_integrity_boundary",
    );
  });

  it("Portal 拒绝接受产品实际可能返回的四个授权结果码", () => {
    const record = completedRecord("linux-kde-wayland");
    const denied = record.results.find((result) => result.id === "auto_paste_denied");
    denied.observedReasonCode = "portal_keyboard_not_granted";

    expect(verifyQaRecord(record).passed).toBe(true);
    denied.observedReasonCode = "wayland_portal_permission";
    expect(verifyQaRecord(record).passed).toBe(false);
  });

  it("每项必须同时有文字观测和证据引用", () => {
    const record = completedRecord("macos-apple-silicon");
    record.results[0].observation = "";
    record.results[1].evidence = [];

    const verification = verifyQaRecord(record);
    expect(verification.passed).toBe(false);
    expect(verification.errors).toContain("clipboard_text 缺少 observation");
    expect(verification.errors).toContain("clipboard_html 缺少 evidence");
  });

  it("Markdown 报告保留 SHA、结论和逐场景证据", () => {
    const record = completedRecord("linux-kde-wayland");
    const verification = verifyQaRecord(record);
    const report = formatQaReport(record, verification);

    expect(report).toContain(`Commit: \`${SHA}\``);
    expect(report).toContain("Result: **PASS**");
    expect(report).toContain("| portal_shortcut_denied | expected_degraded |");
  });
});
