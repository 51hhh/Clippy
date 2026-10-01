# Windows WebM 编译参数修复

需求 ID：`WIN-WEBM-MSVC-01`。
父任务：`WIN-NATIVE-01 / W10`。基线：`fe37aec2e6776814248ce935d917aa46181dd410`。

## Goal

消除 vendored `webm-sys 2.2.1` 向 MSVC 传入 GCC 参数产生的 D9002，保留现有编码器、
WebM C ABI 和各编译器的有效行为。

## Requirements

1. 按 `cc::Tool` 的真实编译器族选择参数；MSVC 和 clang-cl 不接收 `-fno-rtti`、
   `-std=gnu++11`、`-fno-exceptions`。GCC/Clang（包括 Windows GNU）保留现有三个参数。
2. MSVC 沿用之前忽略无效参数后的默认 C++ 模式；不另改 RTTI、异常或 CRT 策略。
3. 仅调整 `build.rs`；保持 libwebm C/C++ 源码、现有 Opus FFI、2.2.1 版本、Cargo path 和许可证。
4. 来源记录保留 crates.io 2.2.1 archive SHA-256 与上游 Git ID，补丁构建脚本按原始字节固定哈希；
   Windows Git 检出保持该文件 LF。不能用归一化输入掩盖源码漂移。
5. 证据区分实际 MSVC 编译命令、Windows 全量 QA 回归、三平台/四原型 CI、桌面/安装 QA；
   后两者未执行时不计作通过。

## Acceptance Criteria

- [x] 原构建脚本在 MSVC 原生重新编译中复现三种 D9002，保存原始命令与返回码。
- [x] 修复后实际 MSVC 命令不含三个 GCC 参数，相关 D9002 消失；没有新增全局告警屏蔽。
- [x] 原始构建脚本哈希漂移的负例被供应链校验拒绝，复原后成功。
- [x] Windows `recording-windows-av-qa` check/clippy/test 与现有 VP9/Opus/WebM 回归通过。
- [ ] 同 SHA Ubuntu/Windows/macOS 原生与四项 codec 原型检查通过，GNU/Clang 参数经原生编译覆盖。
- [x] CHANGELOG、patch 来源、PR 引用同一 ID，未完成的桌面/安装验收保持未完成。

## Out of Scope

- 升级 codec、改变容器/音频轨道行为、默认开放录屏、发布或合入 dev。
- 新增 MSVC RTTI/异常/CRT 策略；压掉 D9002 或降低 Rust clippy 严格度。
- 把编译/合成源回归当作 Windows 真实桌面、有声录屏或安装验收。

## Verification

Windows 11 x64；Rust 1.98.1、`cc 1.2.61`、MSVC 14.44.35207、clang-cl 23.1.2。

- 来源 archive 按 crates.io 的 SHA-256 `4573631d064f24e233a9cd6e5764eef0f364f1248444c0049d047a7784e613a2`
  校验成功；29 个 libwebm C/C++ 文件与原 archive 字节相同。既有许可证只少一个末尾空行，正文一致。
- 在独立临时 Cargo driver（同版本 `cc`、实际 vendored path）开启 `CC_ENABLE_DEBUG_OUTPUT=1`：
  原始 MSVC 六个源文件编译均收到三个 GNU 参数并产生 D9002，check exit 0；补丁六个命令不再
  含这些参数/相关 D9002，check exit 0。保留 `-MD`、`-Z7`、`-Brepro`、`-W0` 等原有有效参数。
- clang-cl 原参数被明确报告为 ignored；前后预处理均为 C++14、RTTI=1、exceptions=0。
  补丁六个真实编译命令通过，check exit 0，无三个 GNU 参数/相关告警。
- 原始 `build.rs` SHA-256 为 `eda0ed58d90a1a492291df6755c91d8f40faa489409f185e6e0db5e27251af6b`；
  补丁为 `c7e90a7f5cd1e24d0cbe355442c21cfaf15d27d13ea0bf64f1d86e37f6b1b833`。
  追加一个 LF 被原始字节校验以 exit 1 拒绝；复原后通过。构建脚本 rustfmt 与 diff whitespace 检查通过。
- 首次全门禁的 checkout 位于另一个 Rust workspace 的 `target` 下，独立 xcap fmt/clippy
  向上找到父 workspace 而失败；默认 Rust 1040 passed、5 ignored，QA 尚未完成时主动终止。
  此次运行不计作完整门禁通过。移到仓库根 `.worktrees` 后独立 vendor fmt 已通过，Node 24.21.0 完整门禁最终为 23 passed、0 failed、1 skipped（Linux smoke）。
  默认 Rust 1040 passed / 5 ignored，QA Rust 1093 passed / 5 ignored；两者不累加。
  前端 74 文件 / 1284 passed；Python 31 项质量与 3 项视觉段落通过。
- 诊断日志在调用项目的 ignored `src-tauri/target/`：`webm-msvc-driver-red.log`、
  `webm-msvc-driver-green.log`、`webm-clang-cl-green.log`、`windows-webm-full-qa*.log`。

本机完整 QA 门禁通过；当时 GNU/Clang 原生 CI、修改后同 SHA CI、桌面/安装验收仍待执行。
父分支 fe37aec 的 Windows 前端 CI 已通过，但 Python 新暴露的问题仍由 WIN-NATIVE-01 修复，
本分支不将该父分支 CI 写为整体通过。

后续父分支更新：同步 `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的 Windows OCR DACL
语义校验与浏览器语料 LF 属性，避免在最新 CI 继续携带父分支已知失败。冲突仅为
`.gitattributes` 末尾，保留 WebM 与四个 OCR 来源属性；C++ 源码和补丁哈希不变。
本机完整录屏 QA 证据绑定 `e4ccc46c3ca22b894ceb18b6c799eedd69358a6c`；同步后的 Python
33 + 3 项另作验证，不能将旧 SHA 的完整门禁写成新 SHA 通过。当时两套原生 CI 仍待终态。

全新 `core.autocrlf=true` clone 在 e4ccc46 首次检出即通过 codec 供应链校验，原始
build.rs 哈希为 c7e90a7f5cd1e24d0cbe355442c21cfaf15d27d13ea0bf64f1d86e37f6b1b833，Git 状态干净。
PR：[14](https://github.com/51hhh/Clippy/pull/14)。

### 原生 CI 与后续依赖基线

- [e4ccc46 run 36815516209](https://github.com/51hhh/Clippy/actions/runs/36815516209) 最终为六项
  success、一项 Windows failure；失败为旧父分支 OCR DACL/来源问题，不能计作全部通过。
- [437949b run 36819267902](https://github.com/51hhh/Clippy/actions/runs/36819267902) 的四项 codec
  原型以及 Ubuntu/Windows 原生成功；macOS 原生在未修改的 OCR 子进程取消夹具失败，
  1054 passed / 1 failed / 5 ignored。这些结果证明 MSVC/GNU/Clang codec 编译回归，
  不证明完整七项成功，也不声称该失败由 C++ 补丁引起。
- 依赖由独立 `OCR-PROC-CANCEL-01` / [PR #15](https://github.com/51hhh/Clippy/pull/15) 修复。
  自动审批最初拒绝跨分支同步，理由为关注点混合；用户随后明确授权调整 #14 的基线到 #15
  并同步依赖。新基线为 `79828665a2d77308b0973b19d993ae745590d521`，#14 相对于它仍限定
  六个原有 WebM 文件；OCR 测试属于继承的基线，不计入 WebM 改动范围。
- CHANGELOG 冲突仅为两个独立条目的插入位置，保留 WebM/OCR 需求段落与最新 Windows 验证记录。
  编码器源码、构建脚本原始哈希和编译策略不变；同步后新 SHA 完整七项 CI 待执行。
