#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const defaultRepositoryRoot = resolve(scriptDirectory, "..");

const REQUIRED_OPTIONS = ["backend", "platform", "signing", "sha", "output"];
const RESERVED_FIELDS = new Set([
  "commit",
  "version",
  "platform",
  "signing",
  "backend",
  "framework",
  "framework_version",
  "renderer",
  "identity",
  "data_namespace",
  "single_instance_key",
  "shortcut_namespace",
  "slint",
  "tauri",
]);

function read(repositoryRoot, path) {
  return readFileSync(resolve(repositoryRoot, path), "utf8");
}

function capture(source, pattern, label) {
  const match = source.match(pattern);
  if (!match) throw new Error(`无法从仓库配置读取 ${label}`);
  return match[1];
}

function readLegacyProfile(repositoryRoot) {
  const appConfig = JSON.parse(read(repositoryRoot, "apps/legacy-tauri/native/tauri.conf.json"));
  const lockfile = read(repositoryRoot, "Cargo.lock");
  return {
    version: appConfig.version,
    backend: "legacy-tauri",
    framework: "Tauri",
    frameworkVersion: capture(
      lockfile,
      /^\[\[package\]\]\nname = "tauri"\nversion = "([^"]+)"$/m,
      "Tauri 版本",
    ),
    renderer: "webview",
    excludedFramework: ["slint", "not included"],
  };
}

function readSlintProfile(repositoryRoot) {
  const manifest = read(repositoryRoot, "apps/slint/Cargo.toml");
  const packager = read(repositoryRoot, "apps/slint/Packager.qa.toml");
  const identity = capture(packager, /^identifier = "([^"]+)"$/m, "Slint QA identity");
  if (!manifest.includes('"renderer-femtovg"')) {
    throw new Error("Slint QA 元数据只支持当前已批准的 FemtoVG renderer");
  }
  return {
    version: capture(manifest, /^version = "([^"]+)"$/m, "Slint 应用版本"),
    backend: "slint",
    framework: "Slint",
    frameworkVersion: capture(manifest, /^slint = \{ version = "=([^"]+)"/m, "Slint 版本"),
    renderer: "femtovg",
    identity,
    dataNamespace: identity,
    singleInstanceKey: identity,
    shortcutNamespace: `${identity}.shortcuts`,
    excludedFramework: ["tauri", "not included"],
  };
}

export function buildQaBuildInfo(options, repositoryRoot = defaultRepositoryRoot) {
  const { backend, platform, signing, sha, renderer, fields = [] } = options;
  if (!/^[0-9a-f]{40}$/i.test(sha ?? "")) throw new Error("sha 必须是 40 位十六进制 commit");
  if (!/^[a-z0-9][a-z0-9-]*$/.test(platform ?? "")) throw new Error("platform 格式无效");
  if (!signing || /[\r\n]/.test(signing)) throw new Error("signing 格式无效");
  if (renderer !== undefined && (
    backend !== "slint" || platform !== "linux-x64" || renderer !== "femtovg+skia-software"
  )) {
    throw new Error("双渲染器元数据仅用于 Linux x64 Slint 对照包");
  }

  const profile = backend === "legacy-tauri"
    ? readLegacyProfile(repositoryRoot)
    : backend === "slint"
      ? readSlintProfile(repositoryRoot)
      : (() => { throw new Error(`不支持的 backend: ${backend}`); })();

  const entries = [
    ["commit", sha.toLowerCase()],
    ["version", profile.version],
    ["platform", platform],
    ["signing", signing],
    ["backend", profile.backend],
    ["framework", profile.framework],
    ["framework_version", profile.frameworkVersion],
    ["renderer", renderer ?? profile.renderer],
  ];
  if (profile.identity) {
    entries.push(
      ["identity", profile.identity],
      ["data_namespace", profile.dataNamespace],
      ["single_instance_key", profile.singleInstanceKey],
      ["shortcut_namespace", profile.shortcutNamespace],
    );
  }
  entries.push(profile.excludedFramework);

  const seen = new Set();
  for (const field of fields) {
    const separator = field.indexOf("=");
    if (separator <= 0) throw new Error(`扩展字段必须是 key=value: ${field}`);
    const key = field.slice(0, separator);
    const value = field.slice(separator + 1);
    if (!/^[a-z][a-z0-9_]*$/.test(key)) throw new Error(`扩展字段名无效: ${key}`);
    if (RESERVED_FIELDS.has(key)) throw new Error(`扩展字段不能覆盖保留字段: ${key}`);
    if (seen.has(key)) throw new Error(`扩展字段重复: ${key}`);
    if (!value || /[\r\n]/.test(value)) throw new Error(`扩展字段值无效: ${key}`);
    seen.add(key);
    entries.push([key, value]);
  }

  return `${entries.map(([key, value]) => `${key}=${value}`).join("\n")}\n`;
}

export function parseArguments(args) {
  const options = { fields: [] };
  for (let index = 0; index < args.length; index += 1) {
    const flag = args[index];
    const value = args[index + 1];
    if (!flag.startsWith("--") || value === undefined || value.startsWith("--")) {
      throw new Error(`参数格式无效: ${flag}`);
    }
    index += 1;
    const key = flag.slice(2);
    if (key === "field") options.fields.push(value);
    else if (key === "renderer" || REQUIRED_OPTIONS.includes(key)) options[key] = value;
    else throw new Error(`未知参数: ${flag}`);
  }
  for (const key of REQUIRED_OPTIONS) {
    if (!options[key]) throw new Error(`缺少参数: --${key}`);
  }
  return options;
}

function main() {
  const options = parseArguments(process.argv.slice(2));
  const content = buildQaBuildInfo(options);
  writeFileSync(options.output, content, { encoding: "utf8", flag: "wx" });
  process.stdout.write(`QA build metadata: ${options.output}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
