import { describe, expect, it } from "vitest";
import {
  isVendoredPathPackage,
  lockedPackageBody,
} from "../../scripts/recording-codec-lock.mjs";

const pathPackage = (newline) =>
  [
    "[[package]]",
    'name = "shiguredo_libvpx"',
    'version = "2026.2.0-canary.1"',
    "dependencies = [",
    ' "bindgen",',
    "]",
    "",
    "[[package]]",
    'name = "yuv"',
    'version = "0.8.19"',
    'source = "registry+https://github.com/rust-lang/crates.io-index"',
    `checksum = "${"a".repeat(64)}"`,
    "",
  ].join(newline);

describe("录制编码供应链锁文件解析", () => {
  it.each(["\n", "\r\n", "\r"])("接受 %j 换行的本地 path 包", (newline) => {
    const cargoLock = pathPackage(newline);
    expect(
      isVendoredPathPackage(cargoLock, "shiguredo_libvpx", "2026.2.0-canary.1"),
    ).toBe(true);
    expect(lockedPackageBody(cargoLock, "yuv", "0.8.19")).toContain(
      "registry+https://github.com/rust-lang/crates.io-index",
    );
  });

  it("拒绝仍带 registry source 的同名包", () => {
    const cargoLock = pathPackage("\r\n").replace(
      "dependencies = [",
      'source = "registry+https://github.com/rust-lang/crates.io-index"\r\ndependencies = [',
    );
    expect(
      isVendoredPathPackage(cargoLock, "shiguredo_libvpx", "2026.2.0-canary.1"),
    ).toBe(false);
  });
});
