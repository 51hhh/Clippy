# Clippy 本地补丁

基于 crates.io **arboard 3.6.1**。保留原 MIT / Apache-2.0 许可证、版本、所有平台及示例源码；生产补丁只涉及 X11。`Cargo.toml.orig` 保留发布包原件，当前 normalized `Cargo.toml` 的测试适配在下文说明。

- 原始 `.crate` SHA-256：`0348a1c054491f4bfe6ab86a7b6ab1e44e45d899005de92f58b3df180b36ddaf`
- 原始 `src/platform/linux/x11.rs` SHA-256：`57058170d748f75df9081dc2d886e8a2fa4fa9ca46af09c37bc5eda36309c053`
- 原件来自 Cargo registry 的同版本包；未引入外部 fork 或改变 public API。

## 写完成屏障

`X11 Inner::write` 对 `set_selection_owner` 返回的实际 `VoidCookie` 调用 `check()`，等待服务器完成写连接请求并传播错误，保留原 `flush()`。原先只有 flush；另一连接 roundtrip 不能证明写连接已完成，watcher 可能先读取内部文本并消费抑制、下一轮才收到 SelectionNotify，从而把 OCR / 译文重新录入历史。

## 大图 INCR

原提供端将整张 PNG 一次写入 X property。高熵 4K / 8K 超过 X server 单次请求上限，独立 arboard 与 xclip 读取均失败。`src/platform/linux/x11/incr.rs` 增加服务事件循环驱动的发送状态机：

- 根据实际 maximum_request_bytes 扣除协议头和 padding 决定 direct / INCR，每块最多 1MiB。等待 requestor 删除属性后再推进，最终零长度块也需确认。
- 最多 4 个活动传输，独立不可变 payload 快照合计 256MiB；同图多 reader 共享 `Arc`。空闲 4s、总 30s 截止，不阻塞服务事件循环等待单个接收者。
- selection 替换和 SelectionClear 保留已接受请求的旧快照；所有目标在分派前统一拒绝活动 `(requestor, property)` 冲突。requestor 销毁只回收其传输，同窗口多 property 的最后一个结束才恢复旧事件 mask；自有窗口销毁才终止服务。
- handover 初始握手最多 4s，开始传输后共享 30s 总预算；不消费由空闲超时提前唤醒。manager 通知到达后仍须等待实际数据及最终零块确认，失败清理残留属性，不能把半张图作为成功结果。
- 接收端先读取零长度属性 metadata，在分配 body 之前检查预算；INCR header 只允许一个 u32，数据按剩余 256MiB 预算读取并拒绝 `bytes_after`。原 10ms 分块截止改为 4s 空闲 / 30s 总截止，兼容合法慢接收链路。
- `image::io::Reader` 改为等价 `image::ImageReader`，仅消除已锁定 image 0.25 的弃用警告。

没有改变 Windows / macOS / Wayland 实现、PNG 编码参数或像素。256MiB 是压缩传输及保留快照预算，不是 PNG 解码像素预算。超时约束针对正常运行 X server 上停滞的 requestor，不提供挂死 X server 的系统 I/O 超时保证。

## 可维护测试门禁

主项目 workspace 显式包含此库、默认成员仅 Clippy；普通 `cargo test` 不运行依赖测试。仅测试用 `env_logger` 从上游 0.10.2 改为复用主锁文件已有的 0.11.10，所有已有生产依赖版本及默认 features 不变。workspace 会锁住原有未启用的可选 `wl-clipboard-rs 0.9.3` 及其 `fixedbitset 0.5.7`、`os_pipe 1.2.3`、`petgraph 0.8.3`、`tree_magic_mini 3.2.2`；它们未加入默认编译图。构建与测试统一使用 `src-tauri/Cargo.lock`，此目录的上游 Cargo.lock 仅保留来源记录，不是 Clippy 门禁锁文件。

上游 `src/lib.rs::tests::{all_tests, multiple_clipboards_at_once}` 标记 ignored，避免显式运行此库的默认单元测试意外访问桌面；没有改变这些测试内容。Linux `SetExtLinux::clipboard` 的剪贴板写入 doctest 改为 `no_run`（仍编译），防止 `cargo test --workspace` 执行示例写入。`.gitignore` 仅增加对原版 examples 的局部保留，避免根研究目录忽略规则遗漏 Cargo 的示例 targets。上游 `autotests=false` 保留；正式协议测试位于主项目 `tests/x11_clipboard_transfer.rs`，由 Cargo 自动发现，文件仅 Linux 编译、所有原生测试默认 ignored 且要求显式隔离环境。

```sh
# 纯状态测试，不连接桌面；3 passed，原生预算测试 ignored。
cargo test --locked --manifest-path src-tauri/Cargo.toml -p arboard --lib platform::linux::x11::incr::tests::
# 新建私有 Xvfb，运行协议、接收预算及 watcher 抑制回归；需要 xvfb / xauth / xclip。
./scripts/test-x11-clipboard.sh
```

脚本接入 `ci-local.sh` 和 Linux CI；Windows / macOS 不执行。覆盖高熵 4K / 8K 的独立进程与 xclip 逐像素互操作、慢 chunk、多 reader、selection 更换、同 key 冲突、销毁 / 不消费回收、完整 handover 及后续小文本。预算 / 快照 / 总截止为纯 unit；写屏障回归覆盖受控写序、内部抑制和外部同内容再次置顶。

详细红绿证据及独立复审见 `.trellis/tasks/09-12-remediation-t10/incr-resolution.md` 和 `review-resolution.md`。升级 arboard 时必须逐项核对写完成、INCR 生命周期及预算边界，并复跑上述门禁后决定移除或重放补丁。
