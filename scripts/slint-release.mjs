#!/usr/bin/env node

import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { resolveReleaseBackend, parseReleaseTag } from "./resolve-release-backend.mjs";

const repository = new URL("../", import.meta.url);
const packagerTemplate = "apps/slint/Packager.toml";
const bundle = "target/packager/slint-release";
const assets = "target/packager/slint-release-assets";

const platforms = {
  "linux-x64": { formats: ["deb", "AppImage"], identifier: "com.clippy.app", minimumSystemVersion: "Ubuntu 22.04", platform: "linux" },
  "windows-x64": { formats: ["exe", "msi"], identifier: "com.clippy.app", minimumSystemVersion: "Windows 10", platform: "windows" },
  "macos-aarch64": { formats: ["dmg"], identifier: "com.clippy.desktop", minimumSystemVersion: "11.0", platform: "macos" },
  "macos-x64": { formats: ["dmg"], identifier: "com.clippy.desktop", minimumSystemVersion: "11.0", platform: "macos" },
};

function absolute(path) {
  return fileURLToPath(new URL(path, repository));
}

function read(path) {
  return readFileSync(absolute(path), "utf8");
}

function packageInfo(tag, platform, sha, signing) {
  const release = checkVersions(tag);
  const profile = platforms[platform];
  if (!profile || !/^[0-9a-f]{40}$/.test(sha ?? "") || !/^[a-z-]+$/.test(signing ?? "")) {
    throw new Error("Expected platform, release SHA and signing mode");
  }
  return {
    commit: sha, version: release.version, platform, signing,
    backend: release.backend, framework: release.framework, framework_version: release.frameworkVersion,
    tauri: "not included", renderer: "femtovg", identity: profile.identifier,
    minimum_system_version: profile.minimumSystemVersion, installer_versions: installVersions(tag),
  };
}

export function installVersions(tag) {
  const { core, prerelease, build } = parseReleaseTag(tag);
  const [major, minor, patch] = core;
  const base = core.join(".");
  let rank = 99n;
  if (prerelease) {
    const match = /^(alpha|beta|rc)\.([1-9]\d?)$/.exec(prerelease);
    if (!match) throw new Error("Slint installers require a prerelease such as alpha.1, beta.1 or rc.1");
    const sequence = BigInt(match[2]);
    const [start, limit] = { alpha: [0n, 29n], beta: [29n, 30n], rc: [59n, 39n] }[match[1]];
    if (sequence > limit) throw new Error("Slint installer prerelease sequence is out of range");
    rank = start + sequence;
  }
  const nativePatch = patch * 100n + rank;
  if (major > 255n || minor > 255n || nativePatch > 65535n) {
    throw new Error("Slint installer version exceeds Windows numeric limits");
  }
  return {
    deb: `${base}${prerelease ? `~${prerelease}` : ""}${build ? `+${build}` : ""}`,
    windows: `${major}.${minor}.${nativePatch}`,
    macosShort: base,
    macosBuild: `${major}.${minor}.${nativePatch}`,
  };
}

export function checkVersions(tag) {
  const release = resolveReleaseBackend(tag);
  if (release.backend !== "slint") throw new Error("Expected a Slint tag");
  for (const path of ["apps/slint/Cargo.toml", packagerTemplate]) {
    const version = /^version = "([^"]+)"$/m.exec(read(path))?.[1];
    if (version !== release.version) {
      throw new Error(`${path} version ${version ?? "(missing)"} does not match tag ${tag}`);
    }
  }
  installVersions(tag);
  return release;
}

function prepare(tag, platform, sha, signing) {
  const info = packageInfo(tag, platform, sha, signing);
  const profile = platforms[platform];
  const versions = info.installer_versions;
  const suffix = profile.platform === "windows" ? ".exe" : "";
  const binaries = absolute("target/packager/slint-binaries");
  mkdirSync(binaries, { recursive: true });
  copyFileSync(absolute(`target/release/clippy-slint${suffix}`), join(binaries, `clippy-app${suffix}`));

  const config = absolute("target/packager/Packager.slint-release.toml");
  const metadata = absolute("target/packager/BUILD-INFO.json");
  writeFileSync(metadata, `${JSON.stringify(info, null, 2)}\n`);
  let template = read(packagerTemplate);
  const version = profile.platform === "macos" ? versions.macosShort
    : profile.platform === "windows" ? versions.windows : versions.deb;
  template = template.replace(/^version = ".*"$/m, `version = "${version}"`)
    .replace(/^identifier = ".*"$/m, `identifier = "${profile.identifier}"`)
    .replace(/"((?:\.\.?\/|third-party-licenses\/)[^"]+)"/g, (_, relative) => {
      const path = resolve(dirname(absolute(packagerTemplate)), relative).replaceAll("\\", "/");
      return JSON.stringify(path);
    }).replace("resources = [", `resources = [\n  { src = ${JSON.stringify(metadata.replaceAll("\\", "/"))}, target = "BUILD-INFO.json" },`);
  if (profile.platform === "macos") {
    writeFileSync(absolute("target/packager/slint-release-info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleVersion</key><string>${versions.macosBuild}</string></dict></plist>
`);
  }
  if (profile.platform === "windows") {
    const thumbprint = process.env.WINDOWS_CERTIFICATE_THUMBPRINT;
    if (!/^[A-F0-9]{40}$/.test(thumbprint ?? "")) throw new Error("Windows signing certificate is missing");
    template += `\ncertificate-thumbprint = "${thumbprint}"\ndigest-algorithm = "sha256"\ntimestamp-url = "http://timestamp.digicert.com"\ntsp = true\n`;
  }
  writeFileSync(config, template);
}

function collect(tag, platform, sha, signing) {
  const info = packageInfo(tag, platform, sha, signing);
  const profile = platforms[platform];
  const bundledInfo = JSON.parse(read("target/packager/BUILD-INFO.json"));
  if (JSON.stringify(bundledInfo) !== JSON.stringify(info)) throw new Error("Bundle provenance does not match the release request");
  const directory = absolute(assets);
  const entries = readdirSync(absolute(bundle), { withFileTypes: true });
  mkdirSync(directory, { recursive: true });
  const packages = [];
  for (const extension of profile.formats) {
    const matches = entries.filter((entry) => entry.isFile() && entry.name.endsWith(`.${extension}`));
    if (matches.length !== 1) throw new Error(`Expected one ${extension} installer, found ${matches.length}`);
    const name = `Clippy_${info.version}_${platform}.${extension}`;
    const path = join(directory, name);
    copyFileSync(join(absolute(bundle), matches[0].name), path);
    const sha256 = createHash("sha256").update(readFileSync(path)).digest("hex");
    packages.push({ name, sha256 });
  }
  writeFileSync(join(directory, `BUILD-INFO-${platform}.json`), `${JSON.stringify({ ...info, packages }, null, 2)}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [action, tag, platform, sha, signing] = process.argv.slice(2);
  if (action === "check") checkVersions(tag);
  else if (action === "prepare") prepare(tag, platform, sha, signing);
  else if (action === "collect") collect(tag, platform, sha, signing);
  else throw new Error("Use check, prepare or collect");
}
