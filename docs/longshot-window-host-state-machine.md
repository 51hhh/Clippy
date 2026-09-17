# 长截图控制窗宿主状态转移基线

本文固定 `capture::longshot::window_host` 在 Phase 6 拆分前的可观察状态机。拆分只移动职责，不改变窗口标签、事件、错误码、JSON wire format、PNG 字节或转移顺序。

## 核心不变量

- registry 同一时间最多拥有一个非 `Empty` slot；任何非空状态都会拒绝第二次 `open`。
- 所有带会话的转移都同时校验控制窗 label 与完整 `LongshotSessionToken`，旧 label、旧 generation 和迟到 worker 不得修改新会话。
- `Active.revealed = false` 只允许 `ready`、deadline、destroy 和强制终结；append、preview、finish 均要求精确且已显示的 Active。
- `Appending` 隐藏窗口后保留旧 snapshot；追加成功才提交新 snapshot，业务失败恢复旧 snapshot，取消失败必须先恢复可见性才能回到 Active。
- `Finishing` 的 `Encoding` 与 `Outputting` 是不同所有权阶段；输出载荷以同一 `Arc<LongshotOutputArtifact>` 判定，不能重新编码后冒充原结果。
- 保存或 Pin 结果不确定时只移除对应重试权限；多个不确定结果的权限收缩只能叠加，不能恢复。
- 窗口已销毁后不产生可交互重试态；成功清理进入 `Empty`，无法证明清理完成进入 `CleanupFailed`。
- `CleanupFailed` 保留占用并阻止新会话；只有准确的补偿成功可以清槽。

## 状态转移表

| 当前状态 | 触发/前置 | 下一状态 | 外部动作与失败收敛 |
|---|---|---|---|
| `Empty` | `open` 校验 caller/selection 并 reserve | `Building` | 记录 overlay 来源并生成唯一 controller label；校验或 build 失败回 `Empty` |
| `Building` | 页面加载路径精确等于 `/longshot-controller.html` | `Pending` | 仅发布 Started barrier；错误路径保持原状态 |
| `Building` / `Pending` | cancel、deadline、destroy、build abort | `Empty` | 关闭或销毁精确窗口；迟到 build 结果不得影响下一会话 |
| `Pending` | `activate` 精确认领 | `Activating(cancel_requested=false)` | 在 blocking worker 中接管普通截图资源 |
| `Activating` | begin 成功且未取消 | `Active(revealed=false)` | 返回 handle/snapshot；等待 `ready` 后显示 |
| `Activating` | cancel/deadline/destroy 先到 | `Activating(cancel_requested=true)` | 先关闭窗口，等待 begin 结果决定是否补偿 |
| `Activating(cancel_requested=true)` | begin 成功 | `Terminating(window_destroyed=true)` | 对刚获得的 token 执行一次 lifecycle cancel；成功后 `Empty` |
| `Activating` | begin 失败 | `Failed(revealed=false)` | 若已请求取消则直接 `Empty`；worker panic 或所有权不确定进入 `CleanupFailed` |
| `Failed` | `ready` | `Failed(revealed=true)` | 显示失败 UI；重复 ready 为幂等 |
| `Failed` | cancel/deadline/destroy | `Empty` | 关闭窗口，不调用 lifecycle cancel |
| `Active(revealed=false)` | `ready` | `Active(revealed=true)` | show + focus；show 失败转入精确终结或 `CleanupFailed` |
| `Active(revealed=false)` | deadline/destroy | `Terminating(window_destroyed=true)` | 精确 cancel；不得恢复已经消失的 UI |
| `Active(revealed=true)` | preview 授权与完成后复核 | 状态不变 | worker 前后都校验精确 owner，迟到结果返回 superseded |
| `Active(revealed=true)` | append 精确认领 | `Appending` | hide → settle → blocking append；保存旧 snapshot 供回滚 |
| `Appending` | append 成功、窗口仍存在且 show 成功 | `Active(revealed=true)` | 提交新 snapshot 并 focus |
| `Appending` | append 业务失败、窗口仍存在且 show 成功 | `Active(revealed=true)` | 恢复旧 snapshot 并返回原错误 |
| `Appending` | cancel | `Terminating(HiddenAppending(NotClaimed))` | cancel 成功后 `Empty`；可重试失败先 show，再回 `Active` |
| `Appending` | destroy 或 show/可见性恢复失败 | `Terminating(window_destroyed=true)` / `CleanupFailed` | 禁止复活窗口；精确执行或保守记录清理失败 |
| `Active(revealed=true)` | finish 首次认领 | `Finishing(Encoding)` | blocking 编码产生单一 artifact；失败可恢复 Active 或补偿 |
| `Finishing(Encoding)` | 编码成功并仍持有精确所有权 | `Finishing(Outputting)` | 同一 `Arc` 进入 copy/save/pin worker |
| `Finishing` | cancel/destroy | `Finishing(window_destroyed=true)` | worker 继续收敛，但不再产生 UI 重试态 |
| `Finishing(Outputting)` | 输出成功 | `Empty` | 返回精确 path 或 pin label；提交一次 |
| `Finishing(Outputting)` | 确定失败且窗口仍存在 | `OutputPending` | 保留同一 artifact 与可用重试集合 |
| `Finishing(Outputting)` | save/pin 结果不确定 | `OutputPending` | 移除相应 action 的重试权限，copy 保留 |
| `Finishing(Outputting)` | 失败且窗口已销毁 | `Empty` | 丢弃 artifact，不恢复 UI，不调用重复 cancel |
| `OutputPending` | 允许的 retry action | `Finishing(Outputting)` | 复用同一 artifact，不重新编码 |
| `OutputPending` | cancel/destroy | `Empty` | 丢弃 artifact；不再操作 lifecycle |
| `Active` / `Appending` | cancel 或 destroy 精确认领 | `Terminating` | 只有认领线程执行 lifecycle cancel |
| `Terminating` | cancel 成功 | `Empty` | 迟到完成对新 generation 无效 |
| `Terminating(RevealedActive)` | 可证明可重试的 cancel 失败且窗口存在 | `Active(revealed=true)` | 保留原 snapshot，返回原业务错误 |
| `Terminating(HiddenAppending)` | 可证明可重试的 cancel 失败且 show 成功 | `Active(revealed=true)` | 恢复旧 snapshot；show 失败进入 `CleanupFailed` |
| `Terminating` | 结果不确定、窗口已毁或补偿失败 | `CleanupFailed` | 保留 token 所有权并阻止第二会话 |
| `CleanupFailed(revealed=false)` | `ready` | `CleanupFailed(revealed=true)` | 展示保守错误状态；重复 ready 幂等 |
| `CleanupFailed` | 精确 emergency compensation 成功 | `Empty` | 只有 exact label/token 可清槽；失败继续占用 |

## Characterization tests 对应域

| 测试域 | 现有代表用例 |
|---|---|
| wire/model | `generation_parser_is_canonical_and_lossless`、`serde_rejects_numeric_generation`、`pin_output_wire_contract_keeps_label_separate_from_save_path` |
| reserve/build/activation | `started_barrier_requires_exact_first_path_and_label`、`deadline_before_publish_revokes_exact_launch_only`、`cancelled_activation_success_is_compensated_without_publishing_active` |
| ready/deadline/destroy | `ready_show_failure_then_destroyed_has_one_termination_winner`、`pending_and_activating_deadlines_execute_exact_destroy_once`、`old_label_events_never_touch_new_reservation` |
| preview | `preview_authorization_is_read_only_and_requires_exact_revealed_active`、`preview_post_check_makes_phase_change_win_over_worker_error` |
| append | `append_executor_orders_hide_settle_worker_show_focus_commit`、`append_cancel_wins_at_every_boundary_without_resurrection`、`append_show_failure_prioritizes_visibility_cleanup_for_all_worker_results` |
| finish/output | `finish_copy_success_orders_effects_and_commits_once`、`finish_output_transitions_require_exact_action_and_arc`、`save_and_pin_uncertainty_compose_to_copy_only_without_reencoding` |
| cancel/cleanup | `explicit_cancel_before_destroyed_keeps_single_winner_and_can_retry_live_failure`、`appending_cancel_failure_reveals_before_retry_or_enters_cleanup_failed`、`emergency_cleanup_success_clears_exact_slot_but_failure_stays_revealable` |

拆分期间每次移动后都运行 `cargo test capture::longshot::window_host`；完整阶段合入前运行 `./scripts/ci-local.sh`，并以同一 SHA 的 Ubuntu、Windows、macOS 原生 CI 作为完成证据。
