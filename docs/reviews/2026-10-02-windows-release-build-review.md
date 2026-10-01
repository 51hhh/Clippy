# WIN-NATIVE-01 / W47 — Windows release 编译审查

冻结被测源码为 `f5ad5da5bfb9cacb92c4cf0f0932f89a31345415`；编译前文档 HEAD `406714bfd46891bda40f4f9b6a8e126d1d2ca9f3`。
本轮分别完成默认 feature 与 `recording-windows-av-qa` 的 Windows x64 release 编译。
两次 native child、wrapper 与实际终端退出码均为 0；编译前后冻结源码干净，随后恢复文档
分支。产物在各自证据目录独立保存；共享 target 中最后一次产物属于默认 feature。
没有产品源码/测试修改，没有测试重跑；W45 完整门禁 30 passed / 0 failed / 1 Linux
smoke skipped、默认 Rust 1193/QA 1256（各 5 ignored）、前端 79/1323 的原计数保持。

## 产物与来源

| 编译图 | EXE 字节 | SHA-256 |
|---|---:|---|
| default | `28747776` | `bde08375a1950f4925e2771f544476947e958a832b2800934e007e85b3837410` |
| qa | `31692800` | `2facc9cb4108101ae7047dae003b321a3bd1d1720324ff57e52fa145977ff644` |

两份 PE 文件均为 AMD64 / Windows GUI，内嵌 Authenticode 证书目录为空。仅解析文件，
未执行/加载 DLL。Cargo 指纹记录 QA 的显式 recording 组合，默认图没有 recording
feature；前端生产构建由每次 Tauri beforeBuildCommand 完成。配置与 Cargo/npm 锁定输入
哈希核对。QA libvpx 固定源归档 SHA-256 `7a479a3c66b9f5d5542a4c6a1b7d3768a983b1e5c14c60a9396edc9b649e015c`
也已核对；Cargo/npm 使用 offline，原 vendored C/C++ 源归档下载单独记录，未宣称整个
原生输入链离线。仅使用已有工具链。

实际主程序 rustc 参数核对 `-C opt-level=s -C panic=abort -C lto=fat -C codegen-units=1
-C strip=symbols`，与 Cargo release profile 一致。此前 check/test 不能替代该 release
优化/链接图；本次构建单独列证据，不增加任何测试通过数。

## panic 清理范围

`WIN-PASTE-CLEANUP-01` 的受控 Click 展开测试在测试 profile 捕获 panic，证明 unwind
作用域退出会清理。release 明确为 `panic=abort`，panic 终止不会执行 Drop，不能将受控
展开用例写成 release 崩溃清理保证。`Result::Err` 的显式释放、普通返回的 Drop 重试与
正常成功路径仍进入本次编译图；它们的实际 Win32 输入行为仍需独立 Native QA。
这项边界记录没有改变原 panic 策略，也没有触发任何系统按键。
[Cargo panic 文档](https://doc.rust-lang.org/cargo/reference/profiles.html#panic)说明测试忽略
panic 设置并使用 unwind；[Rust catch_unwind 文档](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html)
说明该 API 仅捕获展开的 panic。源码、实际 rustc 参数与这两项官方语义分别核对。

## 复核方式与未完成项

两个命令都使用 `tauri build --ci --no-bundle --no-sign --config tauri.ci.conf.json --
--locked --offline`；QA 另加 `--features recording-windows-av-qa`。记录文件含冻结 Git
commit/tree、输入哈希、捕获的 stdout/stderr、非空实际退出码、Cargo feature 指纹和 PE
检查。stdout/stderr 以逐行 UTF-8 文本、Windows 行尾写入后计算文件哈希，未声明为
原生管道字节逐字节快照。旧实际 QA / 模板文件哈希保持；W46 累计审计与 29 项修复历史仍保留。

证据目录：`C:\win\Clippy\src-tauri\target\windows-release-default-build-f5ad5da` 与
`C:\win\Clippy\src-tauri\target\windows-release-qa-build-f5ad5da`，每份
`CODE-VERIFICATION-AUDIT.json` 是已结束编译与只读检查证明。`PE-INSPECTION.json`
保存静态导入及文件结构；这些字段不证明 Windows 10、设备或安装后的运行兼容。

产物未签名、未生成 NSIS/MSI/updater artifact，未启动或安装。当前 SHA CI、Win11
修复后真机/录屏/音频/权限/坐标、Win10/多屏、安装升级卸载、其它宿主完整门禁及
Wayland 回归继续未验。W46 当前 SHA CI 只读查询为零；本轮没有触发远程 CI。
原全局最后两条 Acceptance Criteria 保持未完成，完整目标没有据此闭环。

下一项代码审查：对比已保存的默认/QA PE 导入与源码 API/version 门控、声明的 Windows
基线；仅文件、源码和官方资料核对。桌面停止的用户要求继续有效。
