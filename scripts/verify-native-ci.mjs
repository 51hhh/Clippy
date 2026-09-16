#!/usr/bin/env node

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const REQUIRED_NATIVE_CHECKS = Object.freeze([
  "Check (ubuntu-22.04)",
  "Native Check (windows-latest)",
  "Native Check (macos-latest)",
]);

function checkTimestamp(check) {
  return Date.parse(check.completed_at ?? check.started_at ?? "") || 0;
}

export function evaluateNativeChecks(checkRuns) {
  const checks = [];

  for (const name of REQUIRED_NATIVE_CHECKS) {
    const candidates = checkRuns
      .filter((check) => check.name === name && check.app?.slug === "github-actions")
      .sort((left, right) => checkTimestamp(right) - checkTimestamp(left));
    const latest = candidates[0];
    checks.push({
      name,
      status: latest?.status ?? "missing",
      conclusion: latest?.conclusion ?? null,
      detailsUrl: latest?.details_url ?? null,
      completedAt: latest?.completed_at ?? null,
      passed: latest?.status === "completed" && latest?.conclusion === "success",
    });
  }

  return {
    passed: checks.every((check) => check.passed),
    checks,
  };
}

export function formatNativeCheckReport({ repository, sha, checkedAt, evaluation }) {
  const lines = [
    "# 原生 CI 证据",
    "",
    `- Repository: \`${repository}\``,
    `- Commit: \`${sha}\``,
    `- Checked at: \`${checkedAt}\``,
    `- Result: **${evaluation.passed ? "PASS" : "FAIL"}**`,
    "",
    "| Job | Status | Conclusion | Completed | Evidence |",
    "|---|---|---|---|---|",
  ];

  for (const check of evaluation.checks) {
    const evidence = check.detailsUrl ? `[run](${check.detailsUrl})` : "—";
    lines.push(
      `| ${check.name} | ${check.status} | ${check.conclusion ?? "—"} | ${check.completedAt ?? "—"} | ${evidence} |`,
    );
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

function parseArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value === undefined) {
      throw new Error(`参数必须使用 --name value：${flag ?? "<missing>"}`);
    }
    options[flag.slice(2)] = value;
  }
  return options;
}

function validateRepository(repository) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error(`--repo 必须是 owner/name: ${repository}`);
  }
}

function validateSha(sha) {
  if (!/^[0-9a-f]{40}$/i.test(sha)) {
    throw new Error(`--sha 必须是完整的 40 位 commit SHA: ${sha}`);
  }
}

async function fetchCheckRuns(repository, sha) {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  const headers = {
    Accept: "application/vnd.github+json",
    "User-Agent": "Clippy-native-CI-verifier",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (token) headers.Authorization = `Bearer ${token}`;

  const checkRuns = [];
  for (let page = 1; page <= 10; page += 1) {
    const url =
      `https://api.github.com/repos/${repository}/commits/${sha}/check-runs` +
      `?per_page=100&page=${page}`;
    const response = await fetch(url, { headers });
    if (!response.ok) {
      throw new Error(`GitHub API ${response.status}: ${await response.text()}`);
    }
    const payload = await response.json();
    if (!Array.isArray(payload.check_runs)) {
      throw new Error("GitHub API 响应缺少 check_runs 数组");
    }
    checkRuns.push(...payload.check_runs);
    if (checkRuns.length >= payload.total_count || payload.check_runs.length < 100) break;
    if (page === 10) throw new Error("check-runs 超过 1000 项，拒绝生成不完整证据");
  }
  return checkRuns;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const repository = options.repo;
  const sha = options.sha;
  if (!repository || !sha) {
    throw new Error("用法：node scripts/verify-native-ci.mjs --repo owner/name --sha <40位SHA>");
  }
  validateRepository(repository);
  validateSha(sha);

  const evaluation = evaluateNativeChecks(await fetchCheckRuns(repository, sha));
  const report = formatNativeCheckReport({
    repository,
    sha: sha.toLowerCase(),
    checkedAt: new Date().toISOString(),
    evaluation,
  });
  process.stdout.write(report);
  if (options.output) writeFileSync(resolve(options.output), report, "utf8");
  if (!evaluation.passed) process.exitCode = 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`原生 CI 证据校验失败：${error.message}`);
    process.exitCode = 1;
  });
}
