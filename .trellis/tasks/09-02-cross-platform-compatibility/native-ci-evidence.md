# 原生 CI 与发布证据

- Repository: `51hhh/Clippy`
- Commit: `7dcf9bf28d013ff4ff7991bb2b7b0509453a01fa`
- Tag: `v0.1.18`
- Checked at: `2026-09-03T00:54:07+08:00`
- Result: **PASS**

## CI Check

Run: [`33654963258`](https://github.com/51hhh/Clippy/actions/runs/33654963258)

| Job | Status | Conclusion | Completed | Evidence |
|---|---|---|---|---|
| Check (ubuntu-22.04) | completed | success | 2026-09-02T16:30:14Z | [run](https://github.com/51hhh/Clippy/actions/runs/33654963258/job/100331116265) |
| Native Check (windows-latest) | completed | success | 2026-09-02T16:31:08Z | [run](https://github.com/51hhh/Clippy/actions/runs/33654963258/job/100331116678) |
| Native Check (macos-latest) | completed | success | 2026-09-02T16:30:03Z | [run](https://github.com/51hhh/Clippy/actions/runs/33654963258/job/100331116547) |

## Native QA Packages

Run: [`33654979798`](https://github.com/51hhh/Clippy/actions/runs/33654979798)

| Job | Conclusion | Evidence |
|---|---|---|
| QA Bundle (Linux x64) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33654979798/job/100331417841) |
| QA Bundle (Windows x64) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33654979798/job/100331418170) |
| QA Bundle (macOS Intel) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33654979798/job/100331418425) |
| QA Bundle (macOS Apple-Silicon) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33654979798/job/100331418962) |
| Runtime Smoke (Ubuntu 24.04 x64) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33654979798/job/100333481193) |

Ubuntu 24 job 下载并校验本 run 的 Ubuntu 22 AppImage，在隔离 X11/DBus 环境真实启动后上传冒烟证据。
Windows job 生成临时代码签名证书，仅将公钥短暂加入 `LocalMachine\TrustedPeople`，并要求 NSIS/MSI
的 Authenticode 状态为 `Valid` 且 signer thumbprint 精确匹配。macOS 两个 job 均验证严格 Ad-Hoc
签名、`Signature=adhoc` 与 Mach-O 目标架构。

发布后重新下载 QA 模板与 Ubuntu 24 冒烟证据：九份模板覆盖九个约定 profile，共 287 个场景，
全部绑定上述完整 SHA 与 `0.1.18`；初始状态均为 `not_run`，不会被误判为人工验收通过。Ubuntu 24
证据记录主窗口为可见的 380×500 X11 窗口，单实例回调成功，首帧亮度统计非空白。

四个平台 QA artifact 也已完整下载并在解包目录执行 `sha256sum -c SHA256SUMS.txt`，全部通过：

| 产物 | 字节数 | SHA-256 | QA 签名边界 |
|---|---:|---|---|
| Linux AppImage | 97,491,448 | `bca910129677a1dc6ae0ff9c43920983c5e774c710c7bc9457e2461f3ca67473` | `unsigned-qa-only` |
| Linux DEB | 20,438,436 | `904bed66fe4c00ee4a142d120e4998426e4255b07263bc8f58b2c1ebfe261933` | `unsigned-qa-only` |
| Windows NSIS | 15,191,288 | `fe1a326b69430d0ee1bc19a7f4f94aabc97e1974a90c71bab73b05573f8ee7b3` | `self-signed-qa-only` |
| Windows MSI | 19,066,880 | `32a8c1ce94d48ea39c0c99f6092400c59389ebec83b681dc05a50d7e9150d728` | `self-signed-qa-only` |
| macOS Apple Silicon DMG | 18,794,044 | `fe6a1968ede23d40f2804c107306b85e733c9b012ea8ee4ffbc9a945c2fa7d89` | `ad-hoc-qa-only` |
| macOS Intel DMG | 19,409,323 | `b7641afebaeb5bc3573e6e3c2a75e515835f94af3692f42e19368d48802c9075` | `ad-hoc-qa-only` |

Linux tar 保留 AppImage 执行位；四份 `QA-BUILD.txt` 均精确记录完整 commit、0.1.18、平台和上述签名
边界。结合正式 Release 的四目标 `latest.json`，PRD 中“平台配置、安装包和 updater manifest 与实际
目标匹配”验收项已具备直接产物证据。

离线解包内容审计进一步确认：

- DEB control 为 `clippy 0.1.18 amd64`，依赖使用 Ubuntu 22 可提供的 GTK/WebKitGTK 4.1、GBM、EGL、
  DRM、Wayland EGL 与 Ayatana AppIndicator 包。
- AppImage 的应用 ELF 为 x86-64，最高引用 `GLIBC_2.35`；SquashFS 从实际 offset 944632 解出后没有
  `libwayland-*` 残留，证明 finalization 没把宿主图形 ABI 再捆进去。
- Windows NSIS 外层是标准 32 位安装引导程序，内部 `clippy-app.exe` 为 PE32+ x86-64；MSI metadata
  的 Template 为 `x64;0`、Product/File version 为 0.1.18。
- Apple Silicon 与 Intel DMG 都能完整解开 HFS+ 内容，分别包含 arm64 和 x86_64 Mach-O；两份
  `Info.plist` 均为版本 0.1.18、bundle id `com.clippy.desktop`、最低系统 11.0，并含 `_CodeSignature`。

## Release v0.1.18

Run: [`33656502908`](https://github.com/51hhh/Clippy/actions/runs/33656502908)

Release: [`v0.1.18`](https://github.com/51hhh/Clippy/releases/tag/v0.1.18)

| Job | Conclusion | Evidence |
|---|---|---|
| Check Tag & Version | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336264957) |
| Check CI & Signing Readiness | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336331668) |
| Build (ubuntu22) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336441783) |
| Build (Windows x64) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336441623) |
| Build (macOS Intel) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336441557) |
| Build (macOS Apple-Silicon) | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100336441598) |
| Update Release Notes | success | [run](https://github.com/51hhh/Clippy/actions/runs/33656502908/job/100340283922) |

发布后审计确认：release 不是 draft/prerelease；标签解析到上述完整 SHA；Linux DEB/AppImage、Windows
NSIS/MSI、macOS ARM/Intel DMG 与两架构 updater 包均已上传。`latest.json` 版本为 `0.1.18`，且仅含
`linux-x86_64`、`windows-x86_64`、`darwin-aarch64`、`darwin-x86_64` 四个带非空签名的目标。
发布说明保留 Windows 自签名/SmartScreen 与 macOS Ad-Hoc/未公证警告。
