#!/usr/bin/env node

import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const RELEASE_BACKENDS = Object.freeze({
  LEGACY_TAURI: Object.freeze({
    backend: "legacy-tauri",
    framework: "Tauri",
    frameworkVersion: "2.12.4",
    excludedFramework: "Slint",
  }),
  SLINT: Object.freeze({
    backend: "slint",
    framework: "Slint",
    frameworkVersion: "1.19.0",
    excludedFramework: "Tauri",
  }),
});

const SLINT_VERSION_FLOOR = Object.freeze([0n, 2n, 0n]);
const IDENTIFIER = /^[0-9A-Za-z-]+$/;

function validateIdentifiers(value, { rejectNumericLeadingZero }) {
  if (value === undefined) return;
  for (const identifier of value.split(".")) {
    if (!IDENTIFIER.test(identifier)) {
      throw new Error(`非法 SemVer 标识符: ${identifier || "<empty>"}`);
    }
    if (rejectNumericLeadingZero && /^\d+$/.test(identifier) && identifier.length > 1 && identifier[0] === "0") {
      throw new Error(`SemVer 预发布数字标识符不能有前导零: ${identifier}`);
    }
  }
}
export function parseReleaseTag(tag) {
  if (typeof tag !== "string") throw new Error("release tag 必须是字符串");
  const match = tag.match(
    /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/,
  );
  if (!match) throw new Error(`不支持的 release tag: ${tag}`);

  const [, major, minor, patch, prerelease, build] = match;
  validateIdentifiers(prerelease, { rejectNumericLeadingZero: true });
  validateIdentifiers(build, { rejectNumericLeadingZero: false });

  return Object.freeze({
    tag,
    version: tag.slice(1),
    core: Object.freeze([BigInt(major), BigInt(minor), BigInt(patch)]),
    prerelease: prerelease ?? null,
    build: build ?? null,
  });
}

function compareCore(left, right) {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] < right[index]) return -1;
    if (left[index] > right[index]) return 1;
  }
  return 0;
}

export function resolveReleaseBackend(tag) {
  const parsed = parseReleaseTag(tag);
  const selected =
    compareCore(parsed.core, SLINT_VERSION_FLOOR) >= 0
      ? RELEASE_BACKENDS.SLINT
      : RELEASE_BACKENDS.LEGACY_TAURI;

  return Object.freeze({
    tag: parsed.tag,
    version: parsed.version,
    prerelease: parsed.prerelease,
    build: parsed.build,
    ...selected,
  });
}

export function formatGitHubOutput(release) {
  return [
    `version=${release.version}`,
    `backend=${release.backend}`,
    `framework=${release.framework}`,
    `framework_version=${release.frameworkVersion}`,
    `excluded_framework=${release.excludedFramework}`,
  ].join("\n");
}

function parseArgs(argv) {
  const [tag, ...rest] = argv;
  if (!tag) {
    throw new Error(
      "用法：node scripts/resolve-release-backend.mjs <vMAJOR.MINOR.PATCH> [--github-output <path>]",
    );
  }

  let githubOutput;
  for (let index = 0; index < rest.length; index += 2) {
    const flag = rest[index];
    const value = rest[index + 1];
    if (flag !== "--github-output" || value === undefined) {
      throw new Error(`不支持的参数: ${flag ?? "<missing>"}`);
    }
    githubOutput = value;
  }
  return { tag, githubOutput };
}

function main() {
  const { tag, githubOutput } = parseArgs(process.argv.slice(2));
  const release = resolveReleaseBackend(tag);
  if (githubOutput) {
    appendFileSync(resolve(githubOutput), `${formatGitHubOutput(release)}\n`, "utf8");
  }
  process.stdout.write(`${JSON.stringify(release, null, 2)}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`release 后端解析失败：${error.message}`);
    process.exitCode = 1;
  }
}
