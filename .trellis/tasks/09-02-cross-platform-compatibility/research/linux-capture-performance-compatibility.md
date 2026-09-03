# Linux 截图性能与兼容性结论

## 已复现的发行回归

- v0.1.18 正式 Linux 包由 Ubuntu 22 runner 使用默认 Cargo feature 构建。
- `default = []` 令 `screenshot/screencast.rs` 在正式包中完全不参与编译。
- 当前 GNOME 50.1 Wayland 双屏实测冻结帧约 3.0–4.0 秒，覆盖层首帧约 4.1–6.1 秒。
- GNOME 扩展协议为 v5，但 `Cogl.Texture.get_data` 的 GJS 缓冲没有回写，日志为
  `the pixel buffer never reached Cogl`；因此始终回退到逐屏 PNG。

## 已验证的最优路径

- `448cc25` 引入 Mutter ScreenCast + PipeWire 第一帧取流。
- `0627862` 将其接为 GNOME Wayland 首选后端。
- 同一会话 `RecordMonitor` 所有连接器，提前订阅 `PipeWireStreamAdded`，随后 `Start`；
  每个 node 只消费第一帧，使用映射共享内存并在一次遍历中完成通道重排。
- 不生成 PNG、不写磁盘、不经过 Portal 选择器，画面是每块屏的原生物理像素。
- 2026-09-03 在当前同一台 GNOME 50.1 双屏机器重新运行相同测试：
  `cargo test --features linux-pipewire --lib capture_stage_timings -- --ignored --nocapture`
  得到 `capture_monitor_frames 全程: 228.6 ms`。未启用 feature 的正式路径此前为
  2950.4 ms，实际应用日志为 2993–4004 ms。

## 多屏首帧回归的第二层原因与最终修复

- 将 `96c947a` 的 `screenshot/screencast.rs` 与当前代码逐字对照后，历史取流算法本身没有
  被多平台代码改写；真正的发行回归是 `11d38dc`/`39e8bf3` 把它移出了默认产物。
- 恢复历史代码后，当前机器的内屏仍可在约 52–72 ms 到帧，但外接 HDMI 偶发进入
  PipeWire `streaming/running` 后不触发 process callback。历史实现会等满 1500 ms，随后
  因一块屏缺帧丢弃另一块已经成功的原始帧，再为全部屏幕执行 PNG 兜底。
- PipeWire debug 证明两个源都完成格式协商和 8 个缓冲区分配；客户端每 16 ms 向源 driver
  发送 `RequestProcess`，eDP-1 会产帧，HDMI-1 在 350 ms 内一次也不响应。给客户端增加
  `node.always-process` 也无效，说明问题在 Mutter 的显示器源调度，不在 PNG、像素大小、缓冲
  分配或客户端是否主动触发。
- 最终不再用 `RecordMonitor(connector)` 建源，改成对每块输出的 stage 逻辑矩形调用
  `RecordArea(x, y, width, height)`。Mutter 仍按相交显示器 DPI 输出原生像素，不牺牲清晰度。
  同机同轮 A/B：旧路径约 371–379 ms 且 HDMI-1 连续缺帧；新路径连续 5 轮均拿到
  eDP-1 2560×1600 与 HDMI-1 3840×2160，合计 108–130 ms。
- 350 ms 逐屏兜底继续保留，处理其它 Mutter/PipeWire 运行时异常，但不再是这台机器的正常
  路径。会话清理保持历史 RAII `Stop`，不以省掉 D-Bus 清理换取速度，避免 GNOME 顶栏录制
  指示残留。

## Ubuntu 22/24 依赖边界

- Ubuntu 22 官方仓库提供 `libpipewire-0.3-dev` / `libpipewire-0.3-0` 0.3.48；问题不是
  Jammy 没有 PipeWire。
- 最初 Jammy 编译失败由 Linux 通用依赖 `xcap 0.9 -> pipewire/libspa 0.9` 引入；
  `libspa 0.9.2::VideoInfoRaw::new` 无条件初始化 0.3.65 才加入的字段。
- `pipewire/libspa` 0.7、0.8、0.9、0.10 均存在同类无条件字段初始化，单纯升降版本
  不能同时满足 Jammy 0.3.48 头文件和现有安全 API。
- `11d38dc` 已把 Linux xcap 独立降到 `=0.4.1`，但同时错误地把主动使用的 PipeWire
  ScreenCast 客户端设成可选并默认关闭。两项依赖必须拆开处理。
- 正确方向是在 Jammy 0.3.48 头文件上构建仍包含 ScreenCast 的二进制，再到 Ubuntu
  24/26 验证运行；不得用新版 PPA 头文件伪造 Jammy 兼容。
- PipeWire 客户端最终选择需用真实 Jammy 构建验证：优先评估修复了旧结构字段条件编译的
  新版绑定；若上游仍不兼容，则维护最小适配补丁或隔离小型 helper，而不是删除后端。
- 当前采用的最小方向是保持现有 PipeWire API，对 `libspa` 的 C POD 初始构造维护本地、
  可审计的旧头文件兼容补丁。正式产物仍链接稳定 SONAME `libpipewire-0.3.so.0`，不得把
  构建机的新运行库捆进 AppImage。

## Runtime 兼容策略

- Mutter ScreenCast 是 GNOME 私有接口，运行时调用失败必须带原因退回下一后端，不能让
  私有接口差异阻断截图。当前实现保持 `96c947a` 已验证的调用形状，不新增未经真机证明
  有效的 Version 分支。
- `RecordArea`/`PipeWireStreamAdded` 是共同主路径；`is-recording` 保持历史已验证值，避免
  GNOME 显示至少五秒的共享胶囊。`RecordArea` 自 API v2 即存在，当前使用的
  `is-recording` 属性要求 API v4，与原实现的最低接口要求不变。
- GNOME 支持环境首选 Mutter/PipeWire；wlroots 走 libwayshot；其他 Wayland 走 Portal；
  X11 走 x11rb/xcap。PNG 只保留为带原因的最后兜底。

## 发布与门禁

- 删除“默认 Linux 依赖不得出现 PipeWire”的反向守卫。
- CI 和 Release 必须显式校验正式 Linux 构建包含快速后端。
- 真机 QA 必须保存 `frames_ms`、首块/全部覆盖层 `ready_ms`、实际后端和 fallback reason；
  构建成功或 Xvfb 启动不构成性能验收。
