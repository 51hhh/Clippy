#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriRoot = join(root, "src-tauri");
const vendorRoot = join(tauriRoot, "vendor", "xcap");
const cargoToml = readFileSync(join(tauriRoot, "Cargo.toml"), "utf8");
const cargoLock = readFileSync(join(tauriRoot, "Cargo.lock"), "utf8");
const vendorManifest = readFileSync(join(vendorRoot, "Cargo.toml"), "utf8");
const recorder = readFileSync(
  join(vendorRoot, "src", "windows", "wgc_video_recorder.rs"),
  "utf8",
);
const patches = readFileSync(join(vendorRoot, "PATCHES.md"), "utf8");

const patchedFiles = {
  "src/windows/wgc_video_recorder.rs":
    "cb04e6dfeeb3acd59a7bb2896be6ade13677fd327f59c4c8f5584689396cd068",
  "src/windows/utils.rs": "3a951bdc9860c72536c3f05f1eb769ca6a8ab2e2b7085d9370e70f44b31bd930",
  "src/macos/capture.rs": "14ba152c8a2a9d967d1d9a82eac197a86d5a89995842c8ea4ec13cd117c3e48e",
  "src/macos/impl_window.rs":
    "65a6c9fe1334cbfde0f370ca30df6b64d225a431247a9e604766d8a7388a38e0",
  "src/macos/impl_video_recorder.rs":
    "9e47e283e3fdc506db7fb8766503e911b15d82d3ff84706bf8e4ce7bb976b3fb",
};
for (const [relativePath, expected] of Object.entries(patchedFiles)) {
  const actual = createHash("sha256")
    .update(readFileSync(join(vendorRoot, relativePath)))
    .digest("hex");
  if (actual !== expected) {
    throw new Error(`vendored xcap patched file drifted: ${relativePath}`);
  }
}

if (!/name = "xcap"\nversion = "0\.9\.6"/.test(vendorManifest)) {
  throw new Error("vendored xcap must remain version 0.9.6");
}
if (!cargoToml.includes('xcap = { path = "vendor/xcap" }')) {
  throw new Error("Cargo.toml must patch xcap to the reviewed vendor directory");
}
if (!cargoToml.includes('exclude = ["vendor/libspa", "vendor/xcap"]')) {
  throw new Error("vendored xcap must remain an independent package for explicit native lint");
}
if (!recorder.includes("session.SetIsCursorCaptureEnabled(true)")) {
  throw new Error("vendored xcap WGC recorder must request cursor capture");
}
if (recorder.includes("session.SetIsCursorCaptureEnabled(false)")) {
  throw new Error("vendored xcap WGC recorder must not disable cursor capture");
}
const cursorCalls = recorder.match(/SetIsCursorCaptureEnabled\(/g) ?? [];
if (cursorCalls.length !== 2) {
  throw new Error("xcap WGC cursor patch must keep one call and its one diagnostic string");
}
const windowsUtils = readFileSync(join(vendorRoot, "src", "windows", "utils.rs"), "utf8");
if (!windowsUtils.includes("buffer.as_chunks_mut::<4>().0")) {
  throw new Error("vendored xcap must preserve the reviewed fixed-size BGRA chunk loop");
}
const macCapture = readFileSync(join(vendorRoot, "src", "macos", "capture.rs"), "utf8");
const macWindow = readFileSync(join(vendorRoot, "src", "macos", "impl_window.rs"), "utf8");
const macRecorder = readFileSync(
  join(vendorRoot, "src", "macos", "impl_video_recorder.rs"),
  "utf8",
);
if (
  !macCapture.includes("buffer.as_chunks_mut::<4>().0") ||
  !macCapture.includes("#[allow(deprecated)]") ||
  !macWindow.includes("#[allow(deprecated)]\n        let active_app_dictionary") ||
  (macRecorder.match(/src_row\.as_chunks::<4>\(\)/g) ?? []).length !== 4 ||
  (macRecorder.match(/dst_row\.as_chunks_mut::<[48]>\(\)/g) ?? []).length !== 4
) {
  throw new Error("vendored xcap must preserve the reviewed narrow macOS lint alignment");
}
if (
  !patches.includes("d4ff80928a9758043595f6d9bbda0597efce9e51") ||
  !patches.includes("b6ad471d5ba232bc276382d26a9d3b837d6853b7df389058b5bb1e94dcdd248c") ||
  !patches.includes("920b0a9a1bb13929387f9fe6a14ced1d8f8d8222aba2742449e1fe0ebeb52d45") ||
  !patches.includes("e3bb190867dfcaeb09e1558b0aa08d1e04b8ba9d8fb5f2fe3a4e12d0c22fea53")
) {
  throw new Error("xcap patch provenance hashes are incomplete");
}
readFileSync(join(vendorRoot, "LICENSE"));

const lockedXcap = cargoLock.match(
  /\[\[package\]\]\nname = "xcap"\nversion = "0\.9\.6"\n([\s\S]*?)(?=\n\[\[package\]\]|$)/,
);
if (!lockedXcap || /^(source|checksum) =/m.test(lockedXcap[1])) {
  throw new Error("Cargo.lock must resolve xcap 0.9.6 as the vendored path package");
}

console.log("xcap patch check passed: pinned source, cursor capture, path override, license");
