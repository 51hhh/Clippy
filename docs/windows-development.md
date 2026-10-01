# Windows 开发与验证

当前 review 需求：`WIN-NATIVE-01`，见
[`2026-10-01-windows-native-review.md`](superpowers/plans/2026-10-01-windows-native-review.md)。

## 环境

- Node.js >= 22.12，CI 使用 Node 24；前端依赖使用 `src/package-lock.json`。
- Rust stable，默认 host 为 `x86_64-pc-windows-msvc`，安装 rustfmt、clippy。
- Visual Studio C++ Build Tools 与 Windows SDK；MSVC 链接器和 SDK 必须实际可用。
- WebView2 Runtime；Windows 11 通常已经提供，仍需核对本机安装。
- Python 3，`python.exe` 必须是真实可运行解释器，不能只使用 WindowsApps 的执行别名。

系统前置依赖以 [Tauri 官方 Windows 说明](https://v2.tauri.app/start/prerequisites/#windows) 为准。
本仓库不会通过验证脚本安装系统工具、修改执行策略或启用录屏发布功能。

`.gitattributes` 将 xcap vendor 文本和 Cargo 锁文件固定为 LF，保障原始字节供应链校验。
全新检出可使用 `core.autocrlf=true`；已有检出在拉取属性规则后可能仍保留旧 CRLF 文件。
此时使用全新 checkout 核验，或确认文件无本地修改后按新属性重新检出。校验器始终检查实际字节，
哈希失配需要核对来源，不能通过关闭校验或修改登记哈希来消除错误。

## 本机入口

在仓库根目录的 Windows PowerShell 5.1 或 PowerShell 7 中运行：

```powershell
# 默认构建：Python 合同、Rust、vendor WGC、前端与生产入口
./scripts/ci-windows.ps1

# 最新分支音频/设备选择等录屏 QA 实现：追加非默认 feature 编译、lint 和全量测试
./scripts/ci-windows.ps1 -RecordingQa

# 尚未安装 Rust/MSVC 时只验证前端；明确属于部分门禁
./scripts/ci-windows.ps1 -FrontendOnly

# 跳过生产构建；不能作为完整门禁证据
./scripts/ci-windows.ps1 -Quick
```

执行策略阻止脚本时可在允许本地开发脚本的 PowerShell 环境运行；不需要把系统执行策略改为宽松值。
脚本记录每步通过/失败/跳过，任何执行失败返回非零。`FrontendOnly` 和 `Quick` 的结果不能记为完整
门禁通过。`RecordingQa` 不能与 `FrontendOnly` 组合。

Windows 入口负责 Windows 的本机检查；Linux 的 GNOME/X11/WebKit DOM、Canvas、布局像素 smoke
仍由 `./scripts/ci-local.sh` 执行，跳过项不计为通过。合入前仍须保留完整 Linux 门禁证据及修改后同
SHA 的三项原生 CI，录屏改动另需四项 Recording Codec Prototype；不可用旧 SHA 的结果代替。

## 录屏源码构建附加依赖

`recording-windows-av-qa` 包含固定版本 VP9 源码构建、WGC、WASAPI 与 Opus/WebM。
它不属于默认 feature 或正式 release。

1. 安装 MSYS2 的 `make diffutils perl nasm`，将对应 `usr/bin` 加入当前构建会话 PATH。
2. 将 Visual Studio 的 MSBuild 和 CMake 加入当前会话 PATH；确保可编译 x64 MSVC C/C++。
   Opus 源码构建需要 CMake；可使用 C++ Build Tools 所附版本或独立安装。
3. 安装 LLVM 的 Windows x64 `libclang.dll`，供 VP9 bindgen 使用；`LIBCLANG_PATH` 指向其所在目录。
   Rust 的 `llvm-tools-preview` 只提供符号处理工具，不能替代 libclang。
4. 执行 `rustup component add llvm-tools-preview`。
5. 运行 `./scripts/ci-windows.ps1 -RecordingQa`；libvpx 首次源码构建可能明显慢于默认检查。

已安装工具但当前终端 PATH 尚未刷新时，可在本次会话补齐（MSYS2 如使用自定义目录，应调整路径）：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\msys64\usr\bin;$env:Path"
$vswherePath = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsInstallation = & $vswherePath -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsInstallation) { throw 'Visual Studio C++ Build Tools not found' }
$msbuildDirectory = Join-Path $vsInstallation 'MSBuild\Current\Bin\amd64'
$cmakeDirectory = Join-Path $vsInstallation 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin'
$env:Path = "$msbuildDirectory;$cmakeDirectory;$env:Path"
$env:LIBCLANG_PATH = Join-Path $env:ProgramFiles 'LLVM\bin'
./scripts/ci-windows.ps1 -RecordingQa
```

工具版本与原生流程参考 `.github/workflows/build.yml` 的 Windows Recording Codec Prototype，
不要用普通 `cargo test` 代替该 feature 的编译图。

## 桌面与安装包验收

流程见 [`native-qa.md`](native-qa.md)。Windows 10/11 记录分别生成，不把模板记作通过：

```powershell
node scripts/manual-qa.mjs template --profile windows-11-x64 --sha <完整SHA> --version 0.1.20 --output <记录路径>
node scripts/manual-qa.mjs verify --input <记录路径> --output <报告路径>
```

重点包括混合 DPI/负坐标、跨屏选区与窗口速选、Pin/guide 定位、同权限和管理员目标粘贴、DACL、
系统凭据、默认/非默认音频设备、设备拔出、控制窗排除、暂停/恢复、强杀分段恢复和 30 分钟 A/V 漂移。
安装、升级、卸载、自启动和 updater 使用对应安装包单独记录。

“CI 成功”“本机编译成功”与“桌面验收通过”是独立结论，录屏 QA feature 不因此自动成为发布能力。
