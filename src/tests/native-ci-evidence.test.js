import { describe, expect, it } from "vitest";

import {
  REQUIRED_NATIVE_CHECKS,
  evaluateNativeChecks,
  formatNativeCheckReport,
} from "../../scripts/verify-native-ci.mjs";

function check(name, conclusion = "success", completedAt = "2026-09-02T02:00:00Z") {
  return {
    name,
    status: "completed",
    conclusion,
    completed_at: completedAt,
    details_url: `https://github.com/51hhh/Clippy/actions/runs/${encodeURIComponent(name)}`,
    app: { slug: "github-actions" },
  };
}

describe("原生 CI 证据判定", () => {
  it("只在 Ubuntu、Windows 和 macOS 三项都成功时通过", () => {
    const evaluation = evaluateNativeChecks(REQUIRED_NATIVE_CHECKS.map((name) => check(name)));

    expect(evaluation.passed).toBe(true);
    expect(evaluation.checks.every((item) => item.passed)).toBe(true);
  });

  it.each([
    ["缺少 macOS", REQUIRED_NATIVE_CHECKS.slice(0, 2), "Native Check (macos-latest)", "missing"],
    [
      "Windows 失败",
      REQUIRED_NATIVE_CHECKS.map((name) => check(name, name.includes("windows") ? "failure" : "success")),
      "Native Check (windows-latest)",
      "completed",
    ],
  ])("%s 时失败", (_name, fixture, expectedName, expectedStatus) => {
    const checkRuns = typeof fixture[0] === "string" ? fixture.map((name) => check(name)) : fixture;
    const evaluation = evaluateNativeChecks(checkRuns);

    expect(evaluation.passed).toBe(false);
    expect(evaluation.checks.find((item) => item.name === expectedName)?.status).toBe(expectedStatus);
  });

  it("同名重跑只采用最新结果并忽略第三方 check", () => {
    const runs = REQUIRED_NATIVE_CHECKS.flatMap((name) => [
      check(name, "failure", "2026-09-02T01:00:00Z"),
      check(name, "success", "2026-09-02T02:00:00Z"),
      { ...check(name), app: { slug: "external-ci" }, completed_at: "2026-09-02T03:00:00Z" },
    ]);

    expect(evaluateNativeChecks(runs).passed).toBe(true);
  });

  it("报告包含精确 SHA、逐 job 状态和可点击证据", () => {
    const evaluation = evaluateNativeChecks(REQUIRED_NATIVE_CHECKS.map((name) => check(name)));
    const report = formatNativeCheckReport({
      repository: "51hhh/Clippy",
      sha: "a".repeat(40),
      checkedAt: "2026-09-02T02:30:00.000Z",
      evaluation,
    });

    expect(report).toContain(`Commit: \`${"a".repeat(40)}\``);
    expect(report).toContain("Result: **PASS**");
    for (const name of REQUIRED_NATIVE_CHECKS) expect(report).toContain(`| ${name} |`);
    expect(report).toContain("[run](https://github.com/51hhh/Clippy/actions/runs/");
  });
});
