# 项目审查加固

Requirement ID: `PROJECT-REVIEW-HARDENING-01`

## Goal

修复完整项目审查中确认的供应链、敏感内容、资源边界、联网预览、tmux 集成与重复实现风险，保持现有用户流程和跨平台行为。

## Requirements

- R1：GitHub Actions 第三方 action 必须固定到完整 commit SHA；Actions、npm 与 Cargo 依赖更新入口必须可追踪。
- R2：敏感文本检测必须覆盖常见 token 的嵌入、赋值、HTTP Authorization 与 JSON 形式；重复内容不得从敏感降级为普通。
- R3：剪贴板文本与 HTML 必须有明确的字节上限；列表与实时事件只传输有界摘要，完整内容仅按需读取。
- R4：URL 元数据和站点图标请求必须对实际连接解析结果执行 fail-closed 的公网地址校验，禁止重定向并限制时间、响应大小与并发。
- R5：站点图标必须按页面 URL 正确解析，并在后端验证类型、大小和来源后以本地 data URL 提供给 WebView。
- R6：tmux 捕获必须在启用前备份受影响的用户按键，所有 tmux 命令检查结果，禁用时恢复原配置且不修改全局 hook。
- R7：已知前端与 Rust 依赖漏洞必须升级到修复版本，并在 CI 中持续审计；没有修复版本的警告必须如实保留为未完成边界。
- R8：删除未被产品入口使用的旧剪贴板列表实现及其重复测试，保留 React 列表为唯一实现。

## Acceptance Criteria

- [x] 所有 workflow 的 `uses:` 均为完整 40 位 SHA，并保留可读版本注释；Dependabot 能检查 Actions、前端 npm 与 Rust Cargo 依赖。
- [x] 自动化测试覆盖赋值、Bearer、JSON、普通文本以及敏感标记单向升级。
- [x] 单个文本/HTML 表示超过 1 MiB 时不以该表示入库；列表/事件不含 HTML、图片或超过 2048 Unicode 字符的文本，详情接口仍返回完整允许范围内的内容。
- [x] 自动化测试证明 URL 校验在解析失败、任一私网/保留地址、DNS 重新解析与不安全 favicon 场景下 fail closed；请求总并发有固定上限。
- [x] 自动化测试证明相对 favicon 使用 `Url::join`，仅允许受限图片 MIME/字节，并输出 data URL。
- [x] tmux 单元测试覆盖 shell 引号、备份文件权限、原绑定恢复计划、旧 hook 精确迁移与失败传播；禁用不猜测默认绑定。
- [x] `npm audit` 不再报告已确认的 DOMPurify、source-map-js 与 undici 漏洞；`cargo audit` 不报告 vulnerability，CI 同时执行 npm 与 RustSec 审计。
- [x] 产品源码与测试均不再引用旧剪贴板列表和旧行渲染器。
- [x] `./scripts/ci-local.sh` 完整通过；跳过项、非宿主平台与同 SHA 原生 CI 作为未验证边界记录。

## Out of Scope

- 不改变数据库中已有剪贴板正文的存储格式，不追溯删除历史超大条目。
- 不替换当前 URL 元数据解析器为完整浏览器或通用 HTML 解析服务。
- 不保证第三方网站一定提供可显示的 favicon；不支持 SVG favicon。
- 不验证真实 Windows/macOS 桌面、安装包或远程同 SHA CI；这些继续由原生 CI/QA 流程完成。
- 不把 RustSec 默认允许的 unmaintained/unsound warning 冒充已修复；本轮记录上游边界，但不进行跨框架替换。
- 不修改独立的 `fxxkDJTU/` 项目。

## Decision

**Context**：列表 IPC、外部 URL、tmux 配置与发布 workflow 都位于长期运行或高权限边界，局部兜底不足以阻止资源放大、DNS 重新绑定、用户配置覆盖或供应链漂移。

**Decision**：在数据进入边界先拒绝超限内容，列表只发送投影；联网使用自定义解析器校验实际连接地址并在后端代理 favicon；tmux 用私有备份文件恢复原绑定；workflow 全部固定 SHA。

**Consequences**：超过上限的剪贴板内容会被明确忽略；首次选中列表项会按需读取详情；不安全或无法验证的链接预览/图标会缺省而不是冒险联网；tmux 捕获只管理声明的六个按键。
