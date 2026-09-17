#!/usr/bin/env node

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

function walkRust(directory) {
  const files = [];
  for (const name of readdirSync(directory).sort()) {
    const path = join(directory, name);
    const stat = statSync(path);
    if (stat.isDirectory()) files.push(...walkRust(path));
    else if (name.endsWith(".rs")) files.push(path);
  }
  return files;
}

function stripRustComments(source) {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/.*$/gm, "");
}

function sorted(set) {
  return [...set].sort();
}

function quotedValues(source) {
  return new Set([...source.matchAll(/["']([a-z][a-z0-9_]*)["']/g)].map(match => match[1]));
}

export function collectRustCommandDefinitions(rustSources) {
  const commands = new Set();
  const expression = /#\[tauri::command\](?:\s*#\[[^\]]+\])*\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  for (const source of rustSources.values()) {
    const production = stripRustComments(source);
    for (const match of production.matchAll(expression)) commands.add(match[1]);
  }
  return commands;
}

function generateHandlerBody(source) {
  const marker = "tauri::generate_handler![";
  const start = source.indexOf(marker);
  if (start < 0) throw new Error("找不到 tauri::generate_handler! 注册表");
  let depth = 1;
  for (let index = start + marker.length; index < source.length; index += 1) {
    if (source[index] === "[") depth += 1;
    else if (source[index] === "]") depth -= 1;
    if (depth === 0) return source.slice(start + marker.length, index);
  }
  throw new Error("tauri::generate_handler! 注册表没有闭合");
}

export function collectRegisteredCommands(libSource) {
  const body = stripRustComments(generateHandlerBody(libSource));
  return new Set(
    body
      .split(",")
      .map(entry => entry.trim())
      .filter(Boolean)
      .map(entry => entry.split("::").at(-1)),
  );
}

export function collectLiteralInvokes(apiSource) {
  const commands = new Set();
  const expression = /\binvoke(?:<[^()]*>)?\s*\(\s*(["'`])([^"'`]+)\1/g;
  for (const match of apiSource.matchAll(expression)) commands.add(match[2]);
  return commands;
}

export function collectViewerRequestCommands(apiSource) {
  const match = apiSource.match(/const\s+VIEWER_REQUEST_COMMANDS\s*=\s*\[([\s\S]*?)\]\s*as const\s*;/);
  return match ? quotedValues(match[1]) : new Set();
}

export function collectDynamicInvokeIdentifiers(apiSource) {
  const identifiers = [];
  const expression = /\binvoke(?:<[^()]*>)?\s*\(\s*([^\s"'`][A-Za-z0-9_$]*)/g;
  for (const match of apiSource.matchAll(expression)) identifiers.push(match[1]);
  return identifiers;
}

export function collectAccessCommands(accessSource) {
  const groups = new Map();
  const expression = /const\s+([A-Z][A-Z0-9_]*_COMMANDS):\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/g;
  for (const match of accessSource.matchAll(expression)) groups.set(match[1], quotedValues(match[2]));
  return groups;
}

function missing(left, right) {
  return sorted(new Set([...left].filter(value => !right.has(value))));
}

export function validateContract({ rustSources, libSource, apiSource, accessSource }) {
  const errors = [];
  let definitions;
  let registered;
  try {
    definitions = collectRustCommandDefinitions(rustSources);
    registered = collectRegisteredCommands(libSource);
  } catch (error) {
    return [error instanceof Error ? error.message : String(error)];
  }
  const literalInvokes = collectLiteralInvokes(apiSource);
  const viewerRequests = collectViewerRequestCommands(apiSource);
  const dynamicIdentifiers = collectDynamicInvokeIdentifiers(apiSource);
  const accessGroups = collectAccessCommands(accessSource);
  const accessCommands = new Set([...accessGroups.values()].flatMap(group => [...group]));

  for (const command of missing(definitions, registered)) {
    errors.push(`Rust command 未注册到 generate_handler!: ${command}`);
  }
  for (const command of missing(registered, definitions)) {
    errors.push(`generate_handler! 指向不存在的 #[tauri::command]: ${command}`);
  }
  const frontendCommands = new Set([...literalInvokes, ...viewerRequests]);
  for (const command of missing(frontendCommands, registered)) {
    errors.push(`api.ts invoke 指向未注册命令: ${command}`);
  }
  if (dynamicIdentifiers.join(",") !== "command") {
    errors.push(`api.ts 存在未登记的动态 invoke 参数: ${dynamicIdentifiers.join(",") || "<none>"}`);
  }
  if (!/function\s+viewerInvoke<T>\(command:\s*ViewerRequestCommand\b/.test(apiSource)) {
    errors.push("viewerInvoke 的动态 command 未受 ViewerRequestCommand 联合约束");
  }
  if (viewerRequests.size === 0) {
    errors.push("VIEWER_REQUEST_COMMANDS 为空或无法解析");
  }
  for (const command of missing(accessCommands, registered)) {
    errors.push(`窗口权限矩阵包含未注册命令: ${command}`);
  }
  const viewerAccess = accessGroups.get("IMAGE_VIEWER_COMMANDS") ?? new Set();
  const viewerFrontend = new Set(
    [...frontendCommands].filter(command => command !== "open_image_viewer" && command.includes("viewer")),
  );
  for (const command of missing(viewerFrontend, viewerAccess)) {
    errors.push(`图片查看器前端命令未加入 IMAGE_VIEWER_COMMANDS: ${command}`);
  }
  for (const command of missing(viewerAccess, viewerFrontend)) {
    errors.push(`IMAGE_VIEWER_COMMANDS 没有对应前端调用: ${command}`);
  }
  return errors;
}

export function loadContractSources(repositoryRoot) {
  const rustRoot = join(repositoryRoot, "src-tauri", "src");
  return {
    rustSources: new Map(walkRust(rustRoot).map(path => [path, readFileSync(path, "utf8")])),
    libSource: readFileSync(join(rustRoot, "lib.rs"), "utf8"),
    apiSource: readFileSync(join(repositoryRoot, "src", "js", "api.ts"), "utf8"),
    accessSource: readFileSync(join(rustRoot, "ipc_access.rs"), "utf8"),
  };
}

function main() {
  const here = dirname(fileURLToPath(import.meta.url));
  const repositoryRoot = resolve(here, "..");
  const sources = loadContractSources(repositoryRoot);
  const errors = validateContract(sources);
  if (errors.length > 0) {
    for (const error of errors) console.error(`IPC contract error: ${error}`);
    process.exitCode = 1;
    return;
  }
  const definitions = collectRustCommandDefinitions(sources.rustSources);
  const registered = collectRegisteredCommands(sources.libSource);
  const frontend = new Set([
    ...collectLiteralInvokes(sources.apiSource),
    ...collectViewerRequestCommands(sources.apiSource),
  ]);
  const access = collectAccessCommands(sources.accessSource);
  console.log(
    `IPC contract passed: ${definitions.size} definitions, ${registered.size} handlers, `
      + `${frontend.size} frontend commands, ${access.size} restricted window groups`,
  );
}

const invokedAsScript = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedAsScript) main();
