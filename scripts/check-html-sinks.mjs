#!/usr/bin/env node

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { FRONTEND_API_MODULES } from "./frontend-api-boundary.mjs";

const HTML_SINK_ALLOWLIST = new Map([
  ["src/js/preview/code-renderers.js", { target: "code", sanitizer: "DOMPurify.sanitize", count: 3 }],
  ["src/js/preview/content-renderers.js", { target: "contentEl", sanitizer: "DOMPurify.sanitize", count: 2 }],
]);

const TAURI_API_MODULES = new Set(FRONTEND_API_MODULES);

function walk(directory) {
  const files = [];
  for (const name of readdirSync(directory).sort()) {
    const path = join(directory, name);
    const stat = statSync(path);
    if (stat.isDirectory()) files.push(...walk(path));
    else if (/\.[cm]?[jt]sx?$/.test(name)) files.push(path);
  }
  return files;
}

function lineNumber(source, offset) {
  return source.slice(0, offset).split("\n").length;
}

export function validateFrontendBoundaries(sources) {
  const errors = [];
  const sinkCounts = new Map();
  for (const [path, source] of sources) {
    const tauriImport = /(?:\bfrom\s*|\bimport\s*(?:\(\s*)?|\brequire\s*\(\s*)["']@tauri-apps\//g;
    for (const match of source.matchAll(tauriImport)) {
      if (!TAURI_API_MODULES.has(path)) {
        errors.push(`${path}:${lineNumber(source, match.index)} 只有已登记的 src/js/api 领域模块可导入 @tauri-apps`);
      }
    }
    const tauriGlobal = /\b(?:window|globalThis)\.__TAURI(?:__|_INTERNALS__)/g;
    for (const match of source.matchAll(tauriGlobal)) {
      if (!TAURI_API_MODULES.has(path)) {
        errors.push(`${path}:${lineNumber(source, match.index)} 只有已登记的 src/js/api 领域模块可访问 Tauri global`);
      }
    }

    const forbiddenSink = /\.(outerHTML)\s*=|\binsertAdjacentHTML\s*\(|\bdocument\.write\s*\(|dangerouslySetInnerHTML/g;
    for (const match of source.matchAll(forbiddenSink)) {
      errors.push(`${path}:${lineNumber(source, match.index)} 禁止使用 HTML sink ${match[0]}`);
    }

    const assignment = /\b([A-Za-z_$][\w$]*)\.innerHTML\s*=\s*([^;\n]+)[;]?/g;
    for (const match of source.matchAll(assignment)) {
      const rule = HTML_SINK_ALLOWLIST.get(path);
      const location = `${path}:${lineNumber(source, match.index)}`;
      if (!rule || match[1] !== rule.target || !match[2].includes(rule.sanitizer)) {
        errors.push(`${location} 未登记或未清洗的 innerHTML 写入`);
        continue;
      }
      sinkCounts.set(path, (sinkCounts.get(path) ?? 0) + 1);
    }
  }

  for (const [path, rule] of HTML_SINK_ALLOWLIST) {
    const actual = sinkCounts.get(path) ?? 0;
    if (actual !== rule.count) {
      errors.push(`${path} HTML sink 数量应为 ${rule.count}，实际为 ${actual}`);
    }
  }
  return errors;
}

export function loadFrontendSources(repositoryRoot) {
  const roots = [join(repositoryRoot, "src", "js"), join(repositoryRoot, "src", "react")];
  return new Map(
    roots.flatMap(walk).map(path => [relative(repositoryRoot, path).split("\\").join("/"), readFileSync(path, "utf8")]),
  );
}

function main() {
  const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const sources = loadFrontendSources(repositoryRoot);
  const errors = validateFrontendBoundaries(sources);
  if (errors.length > 0) {
    for (const error of errors) console.error(`Frontend boundary error: ${error}`);
    process.exitCode = 1;
    return;
  }
  const sinkCount = [...HTML_SINK_ALLOWLIST.values()].reduce((sum, rule) => sum + rule.count, 0);
  console.log(
    `Frontend boundary passed: ${sinkCount} sanitized HTML sinks, `
      + `Tauri imports confined to ${TAURI_API_MODULES.size} API domain modules`,
  );
}

const invokedAsScript = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsScript) main();
