# WIN-EXPORT-IDENTITY-01 — Windows 录屏导出同文件保护

## Goal

Windows 导出不能通过大小写、相对父目录、扩展路径或硬链接别名绕过同一源文件保护；
普通新建和覆盖不同目标继续沿用原哈希校验与私有临时文件提交协议。
对应 WIN-NATIVE-01 / W06 的 W39，基线 935766b。

## Requirements

1. 保留原完全相同路径的提前拒绝。Windows 使用实际打开的源/目标句柄判断同一文件，
   比较卷号与完整 128 位 FileId，不按文本大小写或文件内容猜测，不降为 ReFS 不唯一的 64 位 ID。
2. 同文件别名在创建导出临时文件、复制或替换之前拒绝，保留源内容和文件身份；拒绝沿用
   “不能用导出文件覆盖内部恢复产物”。目标不存在时仍允许正常新导出。
3. 不同已有文件即使内容完全相同也可覆盖；目标句柄只查询身份，不增加读取目标全部内容的
   权限要求。不能确认身份的原生错误保留并返回，不能忽略错误继续替换。
4. 原清单/普通文件/长度/哈希检查、私有文件权限、复制上限、封尾同步、替换和失败清理保持；
   不添加依赖/权限或改变 IPC/对话框/录屏默认门控。其它平台不接新身份查询。
5. 在原生产导出函数上用真实临时文件、Windows 句柄和路径别名跑红基线；同组测试修复后
   通过，既有 manifest/导出/媒体/合并合同保留。文件身份证据不是实际桌面对话框或录屏证据。
6. 干净 SHA 完整 Windows 默认/QA 门禁与原文件/断言/源码/日志核对，同 ID 同步文档；
   两图测试重叠不累加。其它文件系统、网络盘和当前 SHA CI/其它宿主保留未验。

## Acceptance Criteria

- [x] 原生产导出函数在同文件 Windows 别名下确有可复现缺口，红基线及同组修复回归核对。
- [x] 大小写、父目录、扩展路径和硬链接别名拒绝，源内容/身份与目录文件集合保持。
- [x] 新建、不同已有目标（包括相同内容）及原哈希失败行为保持，身份错误不执行替换。
- [x] 原合同/固定 API 源码与干净 SHA Windows 默认/QA 门禁核对，新用例包含在各图总数内。
- [x] 同 ID 文档同步，桌面/文件系统矩阵/其它宿主与当前 SHA CI 保留未验。

## Out of Scope

桌面操控保持停止。不运行导出对话框、录屏设备或外部播放器，不安装新包、不运行 Linux/WSL、
不推送/新 PR、合入或发布。不新增任意内部会话目录的覆盖策略，不重构通用私有文件替换。
本轮处理静态同源文件别名；外部进程在检查与提交之间更换目标/父目录、跨进程导出/删除竞态
和 Windows 文件系统/网络盘/权限矩阵仍未验，不能把本机文件测试写成整个 Windows QA 完成。

## Review evidence

export_library_artifact 目前只以 destination == artifact.path 拒绝同文件路径；源哈希正确后
调用原 MoveFileExW 替换目标。原函数红基线确认四者漏过提前检查；三者最后替换报 OS error 5，
硬链接返回 Ok，不将三者描述为成功覆盖源文件。
[Microsoft FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
规定卷号与 128 位 FileId 的句柄身份比较；
[GetFileInformationByHandleEx](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getfileinformationbyhandleex)
提供 FileIdInfo 查询，失败必须读取原错误。既有 windows-sys FileSystem feature 已提供这些声明。

## Validation

原生产导出函数正文不变，仅添加 Windows 测试模块声明，MSVC 红基线原生 child exit 101 /
终端 wrapper exit 1：2 passed / 4 failed / 0 ignored / 1155 filtered。实际 FileIdInfo 证明
四类别名对应源文件：三者迟至提交报 error 5，硬链接返回 Ok。不是提取或替换原导出协议。
同组六项原始测试字节不变，修复后 child/终端 exit 0：6 passed / 0 failed / 0 ignored /
1156 filtered。身份、内容与目录集合保持；普通新建与不同目标（内容相同）成功。新增生产
身份查询错误一项验证真实无效句柄 error 6；这是 helper 单元证据，不是正常文件身份/权限
失败导致整个导出失败的实际复现。既有哈希/替换一项红/绿均通过。
夹具使用原 manifest 数据测试 helper 和真实文件，内容不是可播放 VP9，不计媒体验收。
除 Windows 两处模块声明/一处 guard 调用外，原 manifest tokens 保持；九十六段原函数/
测试正文、原导出哈希/同步/替换/清理和十份关联文件核对；windows-sys 锁定源码与现有依赖
feature 保持。干净源码 69cf0b498a9964789bbce673a0d023aebfd5b0ad 完整 Windows 默认/QA 门禁
确认原生 child/终端 exit 0：30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust
1157 / 5 ignored，QA Rust 1220 / 5 ignored，图谱重叠不累加；新增六项与原生错误一项各图
在总数内。九条关键旧导出/manifest/合并与前轮删除所有权六项按各自图核对；前端 77 文件 /
1307 passed，Python 33 + 3，独立 vendor 十八项和剪贴板二十四项通过；check、严格 lint、
供应链、构建/入口通过。源码/原文件/新测试字节/原始日志、checked helper/门禁脚本/固定
声明哈希及门禁前后干净检出核对；完整门禁证据
C:\win\Clippy\src-tauri\target\windows-export-identity-native-qa-69cf0b4。
其它文件系统/网络盘/权限、外部进程竞态、实际对话框/录屏/播放、其它宿主与当前 SHA CI 留未验。
证据目录 C:\win\Clippy\src-tauri\target\windows-export-identity-contract。
