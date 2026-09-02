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
