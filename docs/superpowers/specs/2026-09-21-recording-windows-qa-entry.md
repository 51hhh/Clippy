# PX-REC-WINDOWS-QA-01 — Windows 受门控录屏入口与真机合同

日期：2026-09-21

关联路线：`PX-REC-01` / `PX-REC-PLAYBACK-01` / `PX-REC-MERGE-01` /
`PX-REC-THUMBNAIL-01`

## Goal

在不改变默认发布能力的前提下，把已经存在的 Windows WGC 帧源、VP9 会话、控制窗原生排除和结果库
连接到显式原型构建，并产出可安装的 Windows 真机 QA 包与结构化验收合同；同一轮也让 Linux X11 QA
包实际包含现有录屏入口，使第一阶段无音频视频闭环可以取得真实桌面证据。

## Requirements

1. 产品入口仅在显式 `recording-vp9-prototype` 能力存在时开放：Linux 只允许原生 X11；Windows 只允许
   `DesktopSession::Native`。默认构建、Linux Wayland、macOS 和未知会话继续隐藏入口并拒绝开始命令。
2. Windows 入口必须复用现有可信 Recording 覆盖层、冻结选区 handoff、WGC 区域帧源、三槽背压、
   VP9/WebM 分段、控制命令和结果库，不新增从前端提交显示器路径、编码器、帧率或光标策略的接口。
3. Windows 10 2004 及以上必须成功应用 `WDA_EXCLUDEFROMCAPTURE` 才能开始；更旧系统继续使用现有
   几何排除计划。任何排除、显示器复核、首帧或控制失败都回滚会话，不能静默录入控制窗。
4. `Native QA Packages` 的 Linux 包显式启用 `recording-vp9-prototype`；Windows 包使用已经固定供应链
   的 `recording-vp9-source-build`，并安装与原型 CI 相同的 MSYS2、NASM、Perl、MSBuild 与 LLVM tools。
   QA 元数据写明 feature；常规 CI、正式 release 和默认 Cargo features 保持不变。
5. 结构化真机合同只把 Linux X11 与 Windows 10/11 标记为原型可测平台，覆盖区域录制、光标、暂停/
   继续/停止、控制窗排除、播放/缩略图/导出，以及强杀后的分段恢复与无损合并。Wayland 与 macOS
   必须记录入口保持关闭，不能用编译成功代替产品可用。
6. 文档必须明确 QA 包属于非默认原型、记录安装包 SHA 与完整 commit，并保留 Windows 光标像素、
   混合 DPI、资源占用和长时间录制为真机证据；未执行结果继续为 `not_run`。

## Acceptance Criteria

- [x] Rust 门控测试证明 Linux X11 和 Windows Native 只在对应 feature/target 开放，其他会话保持关闭。
- [x] Linux 与 Windows QA 构建命令使用正确 feature，Windows source build 工具链与锁定供应链门禁一致。
- [x] 启用录屏 feature 后，即使 benchmark 二进制同时可见，Tauri 仍明确选择 `clippy-app` 作为安装包主程序。
- [x] X11、Windows 和未开放平台的 QA 模板包含各自准确且不可伪造通过的录屏场景。
- [x] 默认构建及 macOS/Wayland 入口没有被扩大，正式 release workflow 没有启用录屏 feature。
- [x] 默认与 VP9 feature 的 check/clippy/test、前端测试、workflow 合同与完整本地门禁通过。
- [ ] 同一 SHA 的 Windows 原生 CI 与 Windows 10/11 真机记录通过后，才可把 Windows 原型入口记为验收。

## Out of Scope

- 不把录屏 feature 设为默认，不修改正式发布包，也不声称 Windows 录屏已经发布可用。
- 不开放 macOS 或 Wayland 产品入口；它们仍分别缺 ScreenCaptureKit 控制窗排除与可取消 Portal 授权 UI。
- 不加入系统音频、麦克风、摄像头、剪辑、转码、全屏 X11 托盘控制或 GPU 编码。
- 不用 GitHub runner、Xvfb、合成帧或交叉编译替代 Windows 10/11 与原生 X11 的实际桌面录制证据。

## Verification

本地验证：

- `npx vitest run tests/manual-qa-contract.test.js tests/regression-guards.test.js`：95 项通过；
- `cargo clippy --all-targets --features recording-vp9-prototype -- -D warnings`：通过；
- 沙箱外 `cargo test --features recording-vp9-prototype`：1063 项通过、15 项忽略；
- `CLIPPY_CROSS_CHECK=1 ./scripts/ci-local.sh`：27 步通过、0 失败、2 跳过；跳过的是缺少
  `cargo-xwin` 的主 crate Windows 交叉 lint 与未请求的 AppImage 可视 smoke，vendor WGC 与 macOS
  arboard 交叉 lint 已通过；
- PyYAML 可解析 `native-qa.yml`，Tauri CLI 帮助确认 `build --features` 是有效参数。

Windows 10/11 与 X11 真机结果必须绑定通过原生 runner 和安装包构建的完整 SHA；当前尚未执行，最后
一项继续保持待验收。

远程验证：

- 提交 `89b68e428588c6a6c988da1ba61c3516a8118de9` 的常规 CI 七个 job 全部通过；
- 同提交首次执行 Native QA 时，Linux 在真正打包阶段暴露多个可用 bin 未声明默认主程序的问题；普通
  `cargo check/clippy/test` 不覆盖该决策。现已在 Cargo package 明确 `default-run = "clippy-app"` 并加入
  静态回归合同，修复后的安装包结果须绑定新的完整 SHA，不能沿用首次失败记录。
