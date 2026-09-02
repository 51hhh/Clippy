#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const UPDATER_PLATFORMS = Object.freeze([
  Object.freeze({
    key: "linux-x86_64",
    config: "src-tauri/tauri.linux.conf.json",
    bundleTarget: "appimage",
    artifactTemplate: "Clippy_{version}_amd64.AppImage",
  }),
  Object.freeze({
    key: "windows-x86_64",
    config: "src-tauri/tauri.windows.conf.json",
    bundleTarget: "nsis",
    artifactTemplate: "Clippy_{version}_x64-setup.exe",
  }),
  Object.freeze({
    key: "darwin-aarch64",
    config: "src-tauri/tauri.macos.conf.json",
    bundleTarget: "app",
    artifactTemplate: "Clippy_{version}_aarch64.app.tar.gz",
  }),
  Object.freeze({
    key: "darwin-x86_64",
    config: "src-tauri/tauri.macos.conf.json",
    bundleTarget: "app",
    artifactTemplate: "Clippy_{version}_x64.app.tar.gz",
  }),
]);

const SEMVER_PATTERN = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const RFC3339_UTC_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

export function artifactName(spec, version) {
  return spec.artifactTemplate.replace("{version}", version);
}

function requireNonEmptyString(value, name) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`${name} 不能为空`);
  }
  return value.trim();
}

function validateInputs({ version, pubDate, baseUrl }) {
  if (!SEMVER_PATTERN.test(version)) {
    throw new Error(`version 不是受支持的 SemVer: ${version}`);
  }
  if (!RFC3339_UTC_PATTERN.test(pubDate) || Number.isNaN(Date.parse(pubDate))) {
    throw new Error(`pubDate 必须是 UTC RFC 3339 时间: ${pubDate}`);
  }

  let parsedUrl;
  try {
    parsedUrl = new URL(baseUrl);
  } catch {
    throw new Error(`baseUrl 不是有效 URL: ${baseUrl}`);
  }
  if (parsedUrl.protocol !== "https:" || parsedUrl.search || parsedUrl.hash) {
    throw new Error("baseUrl 必须是无查询参数和 fragment 的 HTTPS URL");
  }
}

export function buildUpdaterManifest({ version, notes = "", pubDate, baseUrl, signatures }) {
  validateInputs({ version, pubDate, baseUrl });
  const normalizedBaseUrl = baseUrl.replace(/\/+$/, "");
  const platforms = {};

  for (const spec of UPDATER_PLATFORMS) {
    const signature = requireNonEmptyString(signatures?.[spec.key], `${spec.key} signature`);
    const artifact = artifactName(spec, version);
    platforms[spec.key] = {
      signature,
      url: `${normalizedBaseUrl}/${artifact}`,
    };
  }

  return {
    version,
    notes: typeof notes === "string" ? notes.trim() : "",
    pub_date: pubDate,
    platforms,
  };
}

export function readUpdaterSignatures(signatureDir, version) {
  return Object.fromEntries(
    UPDATER_PLATFORMS.map((spec) => {
      const signaturePath = resolve(signatureDir, `${artifactName(spec, version)}.sig`);
      let signature;
      try {
        signature = readFileSync(signaturePath, "utf8");
      } catch (error) {
        throw new Error(`无法读取 ${spec.key} 签名 ${signaturePath}: ${error.message}`);
      }
      return [spec.key, requireNonEmptyString(signature, `${spec.key} signature`)];
    }),
  );
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

function main() {
  const options = parseArgs(process.argv.slice(2));
  for (const required of [
    "version",
    "notes-file",
    "pub-date",
    "base-url",
    "signature-dir",
    "output",
  ]) {
    requireNonEmptyString(options[required], `--${required}`);
  }

  const manifest = buildUpdaterManifest({
    version: options.version,
    notes: readFileSync(resolve(options["notes-file"]), "utf8"),
    pubDate: options["pub-date"],
    baseUrl: options["base-url"],
    signatures: readUpdaterSignatures(resolve(options["signature-dir"]), options.version),
  });
  writeFileSync(resolve(options.output), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`生成 updater manifest 失败：${error.message}`);
    process.exitCode = 1;
  }
}
