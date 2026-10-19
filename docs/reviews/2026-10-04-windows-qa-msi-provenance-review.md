# WIN-QA-MSI-PROVENANCE-01 — Windows QA MSI来源清单审查

日期2026-10-04，W85；关联WIN-NATIVE-01 / W07、WIN-QA-CRT-01。
规范：[WIN-QA-MSI-PROVENANCE-01](../superpowers/specs/2026-10-04-windows-qa-msi-provenance.md)。
先提交规范`2e3fe7b0a6fd8a7c9869e36ea0f29cb1f0e7e397`，修复源码`228cc935bc0767ed9906f03ff5680cc89e72febe`，分支`codex/windows-qa-msi-provenance`。

## 问题与修复

W84原release源码8889192的默认/QA各生成NSIS和MSI，实际native/包装器/终端均0。
W85四包只读解包：包内EXE与原EXE仅Tauri安装格式标记的三个字节不同，原EXE随后恢复。
752份原编译输入与1073份原门禁输入保持，缓存662份文件未改，没有下载或安装新工具。
MSI保存目标目录、文件basename却取源文件名；默认17份基础许可证内容齐全，其中三份
目标文件名与配置名称不同，差异保留，不声称全部资源重命名已修复。

QA十份CRT实际字节齐全，但MSI清单为licenses/PROVENANCE.json，生产部署验证器读取
licenses/windows-qa-vc-runtime.json时exit1/ENOENT；同源码NSIS相同验证器exit0。
这是真实包File表/cabinet与生产文件验证器的失败对照，没有执行安装器或观察loader失败。
修改准备器的staging清单basename为canonical名称，验证器要求同名源文件及原目标映射。
不放宽schema、干净源码、签名/完整版本、原哈希、路径、PE/递归依赖或原部署位置要求。
Rust/前端功能、锁文件/vendor、默认feature、正式发布与基础许可证内容未改。

## 回归及完整门禁

原生产脚本上，旧80项合同通过、新4项失败；修复后同四项回归原字节与原80项共84/0。
覆盖canonical接受、旧名映射拒绝和Windows PowerShell5/7实际准备入口。夹具只模拟
publisher/版本元数据，PE文件不可执行，不加载DLL；实际SDK文件校验单独记录。
原runtime测试正文/超时保留，仅共享夹具适配canonical名；discovery整文件原字节保持。
原源码1073输入中1070保持，另外三份是两准备/校验脚本与runtime测试；752份release输入
中750保持、两包装脚本有明确差异，全部Rust/前端/构建配置原字节相同。

当前干净源码完整Windows11/MSVC、Windows PowerShell5.1 ci-windows.ps1 -RecordingQa
实际native/包装器/终端exit0：33 passed / 0 failed / 1 Linux smoke skipped。
默认Rust1341/QA1493各5ignored；前端81文件/1409，QA recording523含于1493。
旧native通过名称在原各阶段相等；新增四项、定向84项均已含前端总数，重叠图不累加。
代码/部署修复累计52，历史51；平台矩阵、打包、文件比较不计测试通过。

## 修复后的实际QA包文件

使用当前干净源码228cc93准备真实SDK十份CRT与清单，NSIS/MSI再生成和解包。
实际MSI File表及cabinet、NSIS文件均为licenses/windows-qa-vc-runtime.json，旧名不存在；
十份CRT与原SDK哈希相同，清单源/部署哈希相同，两者生产文件验证器均exit0。
应用EXE复用原8889192编译文件，只有Tauri格式标记三个字节变化；明确区分源码/资源准备
SHA228cc93与原EXE编译SHA8889192。这些包不计当前228cc93的release编译成功。

| 包 | 字节数 | SHA-256 |
| --- | ---: | --- |
| QA MSI | 21938176 | `4a7087ddb328ba5293b3997ac6e39cefe270b6421dbb0ed91610364d16d9cd46` |
| QA NSIS | 17276991 | `994d97d63273007c99a704438e250e45584f0e272f6cebb2fa81d54640851c6c` |

证据：windows-qa-msi-provenance-contract、qa-msi-provenance-native-qa-228cc93、
current-package-8889192及current-package-qa-msi-provenance-228cc93-retry2，位于主仓库src-tauri/target。
所有原包、失败日志、原release和W83/W84审计保留各自身份与哈希，不覆盖旧结论。

## 诊断与未完成边界

临时只读MSI脚本无BOM在PS5解析失败，补BOM后查询成功；保存原失败脚本。
baseline原nativeVitest exit1及80/4已保存，随后包装器GBK读取UTF-8摘要报错；按UTF-8
读取原JSON，没有重跑或改写原结果。实际准备器拒绝主仓库target作为worktree输出目录，
改用原允许目录后通过，原路径限制保持。一次默认sandbox写入拒绝后在授权workspace
范围完成；不存在auto-review拒绝。可选旧路径搜索未找到后用实际清单定位。
DARK1059 UI反编译警告保留；File/目录直接只读查询，未声称完整UI反编译或安装界面正确。

当前同SHA release编译、七项CI、其它宿主、安装升级卸载/updater/WebView2运行、无CRT系统
启动、Win10/多屏混合DPI/负坐标、真实WASAPI/WGC/设备/声音/长时同步仍未验。
W83实际8889192远程查询证据保留旧身份，本轮无当前228cc93网络查询/推送/workflow。
旧457包桌面39项2passed/1failed/36not_run不含后续52项修复；不恢复桌面操作。
W53/W59/W63/W67历史失败根因仍未明；W80测试unwind保留不能解释旧失败或证明release强杀恢复。
原8R/9AC/47Tasks与末两条全局AC、原CRT规范末条AC、此规范末项复合AC均保持未完成。
本轮代码和文件阶段完成，WIN-NATIVE-01全局仍未完成。

W86补充：同源码228cc93默认与显式QA release编译及PE/CRT/canonical部署文件核对完成，
不再使用W85的旧EXE复用作为当前release编译证明；W85原包及其8889192 EXE身份不改。
没有打包/签名/安装或设备/桌面运行，原全局未验边界保持。
见 [W86 release](2026-10-04-windows-qa-msi-provenance-release-review.md)。
