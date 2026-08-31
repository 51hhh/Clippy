#!/usr/bin/env bash
# GNOME Shell 扩展的静态检查。
#
# 这份扩展是 include_str! 进二进制的（见 src-tauri/src/capture/shell_extension.rs），
# 里面写错一个字要等到用户注销重登、gnome-shell 加载失败才暴露，而 gnome-shell 的
# 报错只进 journal。所以语法和清单的一致性必须在门禁里就查掉。
#
# 语义层面的两侧契约（uuid、令牌文件名、协议版本、接口名与对象路径）由
# shell_extension.rs 的 embedded_extension_matches_the_uuid_and_token_contract 单测钉住。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UUID="clippy-windows@clippy.local"
EXT_DIR="${REPO_ROOT}/gnome-extension/${UUID}"

[[ -d "$EXT_DIR" ]] || { echo "找不到扩展目录：${EXT_DIR}" >&2; exit 1; }

# GJS 扩展是 ES module。node 只有对 .mjs 才在所有版本上都按 module 解析，
# 因此复制成 .mjs 再检查，避免结果随 CI 的 node 版本漂移。
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
cp "${EXT_DIR}/extension.js" "${TMP_DIR}/extension.mjs"
node --check "${TMP_DIR}/extension.mjs"

node - "$EXT_DIR" "$UUID" <<'NODE'
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

const [extDir, uuid] = process.argv.slice(2);
const metadata = JSON.parse(readFileSync(join(extDir, "metadata.json"), "utf8"));
const problems = [];

// uuid 必须等于目录名：gnome-shell 按目录名查扩展，两者不一致时扩展会被判为无效。
if (metadata.uuid !== uuid) {
  problems.push(`metadata.uuid=${metadata.uuid} 与目录名 ${uuid} 不一致`);
}
for (const field of ["name", "description", "shell-version", "version"]) {
  if (!(field in metadata)) problems.push(`metadata.json 缺字段 ${field}`);
}
if (!Array.isArray(metadata["shell-version"]) || metadata["shell-version"].length === 0) {
  problems.push("shell-version 必须是非空数组");
}
// 只声明支持的版本会让 gnome-shell 直接拒载。本机实测 50.1，至少要覆盖到它。
if (!metadata["shell-version"].includes("50")) {
  problems.push("shell-version 未覆盖 GNOME Shell 50");
}

if (problems.length > 0) {
  console.error(problems.map((line) => `  - ${line}`).join("\n"));
  process.exit(1);
}
console.log(`扩展清单检查通过：${uuid} (shell ${metadata["shell-version"].join(", ")})`);
NODE
