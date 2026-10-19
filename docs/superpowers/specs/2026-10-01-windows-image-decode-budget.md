# WIN-CLIP-IMAGE-BUDGET-01 — Windows 剪贴板图片解码前尺寸预算

## Goal

将 watcher 已有的 16,384 单边 / 40,000,000 像素限制前移到 Windows PNG 与 CF_DIBV5
像素解码之前，避免最终会被拒绝的图片先产生超限像素缓冲区。这是 WIN-NATIVE-01 的独立修复，
基于 cf59157；不改变合法 4K / 8K 图片或像素。

## Requirements

1. 两种 Windows 读取格式共用检查：宽高非零、每边不超过 16,384、总像素不超过 40,000,000。
   检查必须先于 DynamicImage::from_decoder 的像素分配与 read_image；异常输入返回 ConversionFailure。
2. PNG 构造阶段也提供单边尺寸限制；像素总数在取得解码器尺寸后、整图解码前检查。
   保留 8/16-bit PNG 的既有 RGBA8 转换、透明度以及 Chrome/Firefox DIB 兼容处理。
3. 离线故障注入跟踪实际生产解码入口的 read_image 调用；超限用例只允许极小测试缓冲区，
   不尝试真实超大分配、不调用系统剪贴板、不操控桌面。合法 4K、8K 与精确边界只验证尺寸元数据。
4. 本机 Windows 门禁与 Windows Native CI 显式执行依赖库的解码预算回归组；
   默认 Cargo 成员不运行依赖单元测试，不能以主项目测试替代。
   既有 DIB 组作为扩展检查，若修复前已有失败，保留为独立缺陷，不修改或跳过它来宣称通过。
5. 补丁来源、CHANGELOG、审查计划与分层证据同步，保留实际互操作与其它平台未验证项。

## Acceptance Criteria

- [x] 无保护的旧生产解码路径在离线调用顺序合同中失败；修复后超限/零尺寸均在像素解码前拒绝。
- [x] 4K、8K、精确尺寸/像素边界元数据允许；小 PNG 8/16-bit 像素和透明度保持。
- [x] 既有 Chrome/Firefox DIB 扩展回归在后继独立修复 25fb5d7 通过；预算源码 531d791 的 Chrome 失败不改写。
- [x] 完整 Windows 默认/录屏 QA 本机门禁通过，绑定干净源码 SHA；Windows CI 定向入口接线。
- [x] 补丁记录、CHANGELOG 与审查计划同步，未验证边界保留。

W22 后续由独立 WIN-DIBV5-PIXEL-01 修复，原逐像素断言通过，完整 Windows 门禁 27 passed /
0 failed / 1 skipped；见 2026-10-01-windows-dibv5-pixel-offset.md。以下预算源码及其当时失败证据仍保留。

## Out of Scope

整个进程的内存上限、编码剪贴板数据/PNG 元数据的总分配预算、合法 40MP 图片的峰值内存优化、
截图/文件导入解码、其它平台读取策略、写入侧改变、真实系统剪贴板或桌面互操作；不安装新包、
合入 dev 或发布。16-bit 中间像素可能每像素 8 字节，本修复不声称内存最多 160MB。

## Review Evidence

watcher 的 validate_image_layout 在 arboard::get_image 返回后才检查尺寸。锁定 image 0.25.10
的 PngDecoder::new 使用 Limits::no_limits；DynamicImage 的 decoder_to_vec 按 total_bytes
直接创建像素 Vec，只有 usize/isize 表示范围检查。Windows PNG/DIB 原路径均未在此之前约束像素数。
因此已有应用预算挡不住前置超限分配；这是代码层面可达的风险，尚未观察真实桌面 OOM。

## Verification

旧整图解码语句行为不变地提取到共用入口后，五项 Windows MSVC 离线合同 3 passed / 2 failed，
exit 101；超限像素仍调用 read_image，零尺寸也未拒绝。测试只允许四字节缓冲区，未进行大分配。
添加前置检查和两项构造/边界合同后，预算组 7 passed，exit 0。

扩展 image 组 9 passed / 1 failed，exit 101：上游 Chrome DIB 夹具读取 UnexpectedEof。
临时恢复 cf59157 的原 windows.rs 后，同一 Chrome 用例仍失败，exit 101；恢复工作源码后保留全部
原测试和失败日志。不能将此扩展检查写成通过，或把格式错误的拒绝当作像素兼容验收通过。
原测试实际接入依赖的 BmpDecoder；其无文件头 V5 bitfields 读取跳过额外 12 字节值得继续核查，
当前只记录定位线索，尚未修补依赖或证明所有 DIB 提供者受影响。

源码 531d79129128725471288b45a8d0e7a76696b6d4 的完整 Windows 默认/录屏 QA 门禁 exit 0：
25 passed / 0 failed / 1 skipped（Linux smoke），包括九项 CF_HTML 和七项预算定向合同。
默认 Rust 1046 passed / 5 ignored，QA Rust 1099 passed / 5 ignored，前端 75 文件 / 1292 passed；
Rust 两图重叠不累加。源码验证前后检出干净，stdout/stderr 哈希已核对；证据在主检出的
src-tauri/target/windows-image-budget-native-qa-531d791/RESULT.json。
另行 arboard --lib --tests 严格 clippy 通过；CI YAML 的 Windows job/条件及命令已本机解析核对。
门禁只包含七项预算组，不把已知失败的 Chrome 扩展组纳入通过结论，W22 保留未完成。
桌面操作按用户要求停止；
新 SHA 原生 CI、Windows 10、双屏和真实图片互操作保持未验证。
