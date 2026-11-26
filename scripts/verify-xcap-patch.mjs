#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriRoot = join(root, "apps/legacy-tauri/native");
const vendorRoot = join(tauriRoot, "vendor", "xcap");
const workspaceCargo = readFileSync(join(root, "Cargo.toml"), "utf8");
const cargoLock = readFileSync(join(root, "Cargo.lock"), "utf8");
const vendorManifest = readFileSync(join(vendorRoot, "Cargo.toml"), "utf8");
const recorder = readFileSync(
  join(vendorRoot, "src", "windows", "wgc_video_recorder.rs"),
  "utf8",
);
const patches = readFileSync(join(vendorRoot, "PATCHES.md"), "utf8");

const patchedFiles = {
  "src/lib.rs": "6e1992bb6ffb1e907c52bb35a8a8bc4ccf9e7ad2c21685fc704dd2001843c68c",
  "src/window.rs": "e29cacdb676015b539dd69a373fb62db629369837bfd2dea887ec8593f82185d",
  "src/windows/impl_window.rs":
    "ce8533c9ccc81809e4bed2c965177697c94b1390bbb71433acc32d49853c526f",
  "src/windows/wgc_video_recorder.rs":
    "549b0b847d116984d7ebcfc71fb555d754753d5b578aae80353f386793ef5c81",
  "src/windows/mod.rs":
    "516efd4b9d3cef62bd17d4bf4f0f253d33256da6dfe7d3b5ba4c20fc6dbe7b8c",
  "src/windows/wgc_frame_slot.rs":
    "b9856b154564f5a825808076c056d28def32c428960189dcbde49956a9ffa3d5",
  "src/windows/wgc_region_reader.rs":
    "acad132b6f6313c98530c6e84856594d22e55055bc56aac6c15fa8951f3469f2",
  "src/windows/registry_build.rs":
    "ca8d56329161c39d46c7a0756f7ce060f999a4d95e5ce436012afdd8b2b7bc6f",
  "src/windows/wgc_init.rs":
    "7a9d853b5c534c334e74b93eedffc80be23f7f463d995bfadb5515c9733083cb",
  "src/windows/wgc_runtime.rs":
    "be621c84cfa8991bb35be1fcc0aa30bb6442bdbe15b3062eb6d1bebb2445e33a",
  "src/windows/utils.rs": "e8c6677fbb9148d2e9cfb421e56e5f4d98addca4f8c50f1cb08a6faf9f55834f",
  "src/macos/capture.rs": "14ba152c8a2a9d967d1d9a82eac197a86d5a89995842c8ea4ec13cd117c3e48e",
  "src/macos/impl_window.rs":
    "ac6a32a909144d4b2a1686becef2ae592cddba6e6e1ca433477104c6f073c6a6",
  "src/macos/impl_video_recorder.rs":
    "09c964ba782d1f5cd8b170b1e2a6a239f5de84d4a4cc4c71ed89bee18896d742",
  "src/macos/impl_monitor.rs":
    "21ae2af1fc1fba3e206948bbb5088678a78d6d780a79870a4b35c7dffbf52e0b",
  "src/monitor.rs":
    "2e9b14fa6c6185f98c623418368fd92415b040e0e16871aaac5bb0a938b3accc",
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
if (
  !workspaceCargo.includes(
    'xcap = { path = "apps/legacy-tauri/native/vendor/xcap" }',
  )
) {
  throw new Error("workspace Cargo.toml must patch xcap to the reviewed vendor directory");
}
if (
  !workspaceCargo.includes('"apps/legacy-tauri/native/vendor/libspa"') ||
  !workspaceCargo.includes('"apps/legacy-tauri/native/vendor/xcap"')
) {
  throw new Error(
    "vendored libspa and xcap must remain independent packages for explicit native lint",
  );
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
const macMonitor = readFileSync(
  join(vendorRoot, "src", "macos", "impl_monitor.rs"),
  "utf8",
);
const publicMonitor = readFileSync(join(vendorRoot, "src", "monitor.rs"), "utf8");
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
  !macRecorder.includes("input.setCropRect(crop_rect)") ||
  !macRecorder.includes("input.setScaleFactor(scale_factor)") ||
  !macMonitor.includes("pub fn video_recorder_region(") ||
  !macMonitor.includes(".all(f64::is_finite)") ||
  !macMonitor.includes("x + width > monitor_width") ||
  !macMonitor.includes("y + height > monitor_height") ||
  !publicMonitor.includes("pub fn video_recorder_region(")
) {
  throw new Error("vendored xcap must preserve the reviewed macOS region recorder contract");
}
if (
  !patches.includes("d4ff80928a9758043595f6d9bbda0597efce9e51") ||
  !patches.includes("b6ad471d5ba232bc276382d26a9d3b837d6853b7df389058b5bb1e94dcdd248c") ||
  !patches.includes("920b0a9a1bb13929387f9fe6a14ced1d8f8d8222aba2742449e1fe0ebeb52d45") ||
  !patches.includes("e3bb190867dfcaeb09e1558b0aa08d1e04b8ba9d8fb5f2fe3a4e12d0c22fea53") ||
  !patches.includes("332b10d02307519d40c156cca67b63961c5b2856edd1cb2a2b6836cde33090c1") ||
  !patches.includes("0c932e63a93a7e7ee888c303de18762376a3f0c5b4464951198d018eab66a462") ||
  !patches.includes("f5bd15e96b8605431cadc4e75b55b1fc9fe06a6ea4f87d5a902f3d4abc5f8328")
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

console.log(
  "xcap patch check passed: pinned source, WGC region reader and cursor capture, macOS region recorder, window snapshots, path override, license",
);
