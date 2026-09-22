#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isVendoredPathPackage, lockedPackageBody } from "./recording-codec-lock.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tauriRoot = join(root, "src-tauri");
const vendorRoot = join(tauriRoot, "vendor", "shiguredo_libvpx");
const buildScript = readFileSync(join(vendorRoot, "build.rs"), "utf8");
const bindingSource = readFileSync(join(vendorRoot, "src", "lib.rs"), "utf8");

const pinnedArchives = {
  "ubuntu-22.04_x86_64": "4c7a10b8d6f6e3331d55d8d4aaf2f6b942f6da52c0ecc1a8b946251536879c70",
  "ubuntu-24.04_x86_64": "8175f600d2f44fa917fbcfd363c0e3ecf9427cd98c6d0ce97bd6b9ed71dc1141",
  "ubuntu-26.04_x86_64": "fa42cae01b70b608c725a03fda03e6434d3f2597e93aa7210afd80ff16232c2b",
  windows_x86_64: "10c060d5c6d794ee74d227e8717e9237acf799c37d4309111839c80ce5f44eb9",
  macos_arm64: "9dd747b7d734ca9a6d44c2c861e32d5d041e1babc87b58f816b2ffb0ac66b90e",
};
const pinnedSource = {
  url: "https://github.com/webmproject/libvpx/archive/refs/tags/v1.16.0.tar.gz",
  sha256: "7a479a3c66b9f5d5542a4c6a1b7d3768a983b1e5c14c60a9396edc9b649e015c",
};

for (const [target, sha256] of Object.entries(pinnedArchives)) {
  if (!buildScript.includes(`"${target}"`) || !buildScript.includes(`"${sha256}"`)) {
    throw new Error(`vendored libvpx build script is missing the reviewed ${target} archive hash`);
  }
}
if (buildScript.includes("sha256_url") || buildScript.includes("failed to download SHA256")) {
  throw new Error("vendored libvpx must not download its checksum beside the archive");
}
if (!buildScript.includes(`"${pinnedSource.url}"`) || !buildScript.includes(`"${pinnedSource.sha256}"`)) {
  throw new Error("vendored libvpx build script is missing the reviewed source archive input");
}
if (buildScript.includes('Command::new("git")') || buildScript.includes('arg("clone")')) {
  throw new Error("vendored libvpx source build must not clone a mutable Git tag");
}
if (
  !buildScript.includes('(\"x86_64\", \"msvc\") => build_from_source_windows_msvc(src_dir)') ||
  !buildScript.includes('x86_64-win64-vs17') ||
  !buildScript.includes('vpxmd.lib') ||
  !buildScript.includes('make install (MSVC)') ||
  !buildScript.includes('tag_content WholeProgramOptimization false') ||
  !buildScript.includes('unexpected libvpx MSVC project generator')
) {
  throw new Error("vendored libvpx must build a native MSVC archive for Windows MSVC targets");
}
if (!buildScript.includes('(\"windows\", \"x86_64\") if target_env == \"gnu\"')) {
  throw new Error("the upstream Windows prebuilt must remain restricted to the GNU ABI");
}
if (!bindingSource.includes("pub lag_in_frames: Option<usize>")) {
  throw new Error("vendored libvpx must preserve the reviewed zero-lag recording configuration");
}

// actions/checkout 在 Windows 上可能把文本检出为 CRLF；先统一换行符，确保
// 多行 feature 边界在各平台执行同一份已审查合同。
const cargoToml = readFileSync(join(tauriRoot, "Cargo.toml"), "utf8").replace(/\r\n?/g, "\n");
if (!cargoToml.includes('shiguredo_libvpx = { path = "vendor/shiguredo_libvpx" }')) {
  throw new Error("Cargo.toml must patch shiguredo_libvpx to the reviewed vendor directory");
}
if (
  !cargoToml.includes(
    'recording-vp9-source-build = ["recording-vp9-prototype", "shiguredo_libvpx/source-build"]',
  )
) {
  throw new Error("Cargo.toml must expose the reviewed VP9 source-build feature");
}
if (
  !cargoToml.includes('default = ["linux-pipewire", "longshot-wayland-auto"]') ||
  !cargoToml.includes(
    'longshot-wayland-auto = ["ashpd/raw_handle", "ashpd/wayland", "dep:raw-window-handle"]',
  )
) {
  throw new Error(
    "Cargo.toml must keep the product Wayland longshot parent-handle dependencies explicit and enabled",
  );
}
if (
  !cargoToml.includes('"recording-vp9-prototype",\n    "ashpd/raw_handle",\n    "ashpd/wayland",\n    "dep:raw-window-handle",')
) {
  throw new Error(
    "Cargo.toml must keep the Wayland QA entry and native parent-handle dependencies feature-gated",
  );
}
if (
  !cargoToml.includes(
    'yuv = { version = "=0.8.19", default-features = false, features = ["avx", "sse", "rdm", "professional_mode"], optional = true }',
  )
) {
  throw new Error("Cargo.toml must pin the reviewed professional-precision yuv SIMD dependency");
}
if (
  !cargoToml.includes(
    'recording-opus-webm = ["recording-vp9-prototype", "dep:opusic-c", "dep:opusic-sys"]',
  ) ||
  !cargoToml.includes(
    'opusic-c = { version = "=1.6.1", default-features = false, optional = true }',
  ) ||
  !cargoToml.includes(
    'opusic-sys = { version = "=0.7.5", default-features = false, features = ["bundled"], optional = true }',
  )
) {
  throw new Error("Cargo.toml must pin the reviewed bundled libopus dependency graph");
}
if (!cargoToml.includes('recording-linux-av-qa = ["recording-opus-webm"]')) {
  throw new Error("Cargo.toml must keep Linux PipeWire A/V QA behind the reviewed codec feature");
}
if (
  !cargoToml.includes(
    'recording-windows-av-qa = [\n    "recording-vp9-source-build",\n    "recording-windows-audio",\n    "recording-opus-webm",\n]',
  )
) {
  throw new Error("Cargo.toml must keep Windows A/V QA behind the reviewed codec feature set");
}
if (
  !cargoToml.includes(
    'recording-macos-av-qa = [\n    "recording-macos-screencapturekit",\n    "recording-opus-webm",\n    "dep:objc2-core-audio-types",\n    "objc2-core-media/objc2-core-audio-types",\n]',
  ) ||
  !cargoToml.includes(
    'objc2-core-audio-types = { version = "=0.3.2", optional = true }',
  )
) {
  throw new Error(
    "Cargo.toml must keep macOS A/V QA and CoreAudioTypes behind the reviewed feature set",
  );
}
const recordingInfoPlist = readFileSync(join(tauriRoot, "Info.recording.plist"), "utf8");
if (
  !recordingInfoPlist.includes("<key>NSScreenCaptureUsageDescription</key>") ||
  !recordingInfoPlist.includes("<key>NSMicrophoneUsageDescription</key>")
) {
  throw new Error("macOS recording QA plist must declare screen capture and microphone usage");
}
if (
  !cargoToml.includes('webm = { path = "vendor/webm" }') ||
  !cargoToml.includes('webm-sys = { path = "vendor/webm-sys" }')
) {
  throw new Error("Cargo.toml must patch webm and webm-sys to the reviewed vendor directories");
}
const cargoLock = readFileSync(join(tauriRoot, "Cargo.lock"), "utf8");
if (!isVendoredPathPackage(cargoLock, "shiguredo_libvpx", "2026.2.0-canary.1")) {
  throw new Error("Cargo.lock must resolve shiguredo_libvpx as the vendored path package");
}
const lockedYuv = lockedPackageBody(cargoLock, "yuv", "0.8.19");
if (
  !lockedYuv ||
  !/^source = "registry\+https:\/\/github\.com\/rust-lang\/crates\.io-index"$/m.test(
    lockedYuv,
  ) ||
  !/^checksum = "[0-9a-f]{64}"$/m.test(lockedYuv)
) {
  throw new Error("Cargo.lock must pin the reviewed yuv 0.8.19 registry package and checksum");
}
const registryPackages = [
  ["opusic-c", "1.6.1", "89f8e9c909466f15e60277212cc4fec082c68a5e1c9f6e373eee716fec2fed47"],
  ["opusic-sys", "0.7.5", "c9d1ecdf206421bc74343ab3bb2f30ad2abbfee41fa341f7181fecbaf957769a"],
];
for (const [name, version, checksum] of registryPackages) {
  const packageBody = lockedPackageBody(cargoLock, name, version);
  if (
    !packageBody ||
    !/^source = "registry\+https:\/\/github\.com\/rust-lang\/crates\.io-index"$/m.test(
      packageBody,
    ) ||
    !packageBody.includes(`checksum = "${checksum}"`)
  ) {
    throw new Error(`Cargo.lock must pin the reviewed ${name} ${version} registry checksum`);
  }
}
for (const name of ["webm", "webm-sys"]) {
  if (!isVendoredPathPackage(cargoLock, name, "2.2.1")) {
    throw new Error(`Cargo.lock must resolve ${name} 2.2.1 as the vendored path package`);
  }
}

const webmPatch = readFileSync(join(tauriRoot, "vendor", "webm", "PATCHES.md"), "utf8");
const webmSysPatch = readFileSync(join(tauriRoot, "vendor", "webm-sys", "PATCHES.md"), "utf8");
const webmSegment = readFileSync(
  join(tauriRoot, "vendor", "webm", "src", "lib", "mux", "segment.rs"),
  "utf8",
);
const webmFfi = readFileSync(join(tauriRoot, "vendor", "webm-sys", "ffi.cpp"), "utf8");
for (const marker of [
  "set_audio_codec_delay",
  "set_audio_seek_pre_roll",
  "set_timecode_scale",
  "add_frame_with_discard_padding",
]) {
  if (!webmSegment.includes(marker)) {
    throw new Error(`vendored webm is missing reviewed Opus API: ${marker}`);
  }
}
for (const marker of [
  "mux_segment_set_audio_codec_delay",
  "mux_segment_set_audio_seek_pre_roll",
  "mux_segment_set_timecode_scale",
  "mux_segment_add_frame_with_discard_padding",
]) {
  if (!webmFfi.includes(marker)) {
    throw new Error(`vendored webm-sys is missing reviewed Opus FFI: ${marker}`);
  }
}
if (!webmPatch.includes("standards-compliant Opus-in-WebM") || !webmSysPatch.includes("No bundled")) {
  throw new Error("vendored WebM patch manifests must describe the reviewed Opus surface");
}

const licenseFiles = [
  "shiguredo_libvpx-2026.2.0-canary.1.txt",
  "webm-2.2.1-MPL-2.0.txt",
  "libwebm-2.2.1-BSD-3-Clause.txt",
  "libvpx-1.16.0-BSD-3-Clause.txt",
  "yuv-0.8.19-BSD-3-Clause.txt",
  "recording-vp9-prototype-NOTICE.md",
  "opusic-c-1.6.1-BSD-3-Clause.txt",
  "opusic-sys-0.7.5-libopus-1.6.1-BSD-3-Clause.txt",
  "recording-opus-webm-NOTICE.md",
];
const resources = JSON.parse(readFileSync(join(tauriRoot, "tauri.conf.json"), "utf8")).bundle
  .resources;
for (const file of licenseFiles) {
  readFileSync(join(tauriRoot, "third-party-licenses", file));
  if (!Object.hasOwn(resources, `third-party-licenses/${file}`)) {
    throw new Error(`Tauri bundle is missing the recording codec license resource: ${file}`);
  }
}

console.log(
  "Recording codec supply-chain check passed: vendored bindings, pinned codec inputs, licenses",
);
