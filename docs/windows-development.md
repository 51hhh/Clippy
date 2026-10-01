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

Windows 入口还显式运行 `cargo test --locked -p arboard --lib platform::windows::html::tests`。
该组只验证离线 CF_HTML 字节与生产安全解析接线，不读写系统剪贴板。默认 Cargo 成员不运行
依赖库单元测试，普通 `cargo test` 不能替代这组合同；Windows Native CI 同样显式执行。

同一入口显式运行 `cargo test --locked -p arboard --lib platform::windows::image_limits::tests`，
验证 PNG / DIB 共用入口的解码前尺寸预算、PNG 构造限制及小 8/16-bit PNG 像素。
七项均为离线夹具；4K/8K 只验证元数据，不产生大分配。上游 `image_data::chrome_dibv5`
曾在 cf59157 / 531d791 失败，W22 的独立显式偏移修复保留原夹具和逐像素断言。
门禁还显式运行 `platform::windows::image_data::` 五项及 `platform::windows::dib::tests`
三项文件视图合同，全部离线；原失败记录保留，真实提供者互操作仍需单独验收。

## WGC 关闭合同

默认 Windows 门禁还显式执行独立 xcap 包的
`cargo test --locked --manifest-path vendor/xcap/Cargo.toml --lib --features wgc platform::wgc_runtime::tests`。
六项仅用关闭回调验证 WGC 部分失败、成功状态、重试和 Drop 清理，不创建捕获对象或显示器。
WGC clippy 包含 lib tests；主应用 cargo test 不运行依赖库单元测试，不能替代此组。
Windows Native CI 同样显式执行；上游显示器测试保留过滤，不执行完整 vendor test。

`WIN-WGC-INIT-ROLLBACK-01` 另显式执行
`cargo test --locked --manifest-path vendor/xcap/Cargo.toml --lib --features wgc platform::wgc_init::tests`。
四项使用真实 scopeguard 和受控泛型资源，验证注册/session 错误前 Close、原错误及成功转移；
不创建 WinRT 对象。关闭六项与初始化四项独立于应用 Rust 总数，全部由含测试的 vendor clippy 检查。

## 构建号读取边界

`WIN-REGISTRY-BUFFER-01` 在本机/Windows Native CI 显式执行独立 vendor 组：
`cargo test --locked --manifest-path vendor/xcap/Cargo.toml --lib --features wgc platform::registry_build::tests`。
八项使用与原生读取共用的初始化缓冲区/解析入口，验证字节单位、返回范围和错误回退，不访问
注册表、屏幕或捕获对象。该模块不受 wgc feature 门控；使用既有 vendor 测试图减少重复构建。
主应用 cargo test 不运行依赖单元测试，这八项不计入应用总数。上游显示器测试仍过滤；
含测试的 vendor clippy 编译此组。八项构建号与十项 WGC 合同是十八项独立离线 vendor 测试。

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

`WIN-QA-CRT-01`：录屏 QA 的 C++ 依赖需要应用本地运行库。干净检出后先从已有
Visual Studio 的标准 desktop x64 release redist 准备资源；脚本检查 Microsoft 签名、版本、
架构、哈希及 direct/delay import 递归依赖，不安装系统运行库：

```powershell
$runtime = ./scripts/prepare-windows-qa-runtime.ps1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'QA runtime preparation failed' }
npx --prefix src tauri build --ci --target x86_64-pc-windows-msvc --features recording-windows-av-qa `
  --config src-tauri/tauri.windows.conf.json --config src-tauri/tauri.ci.conf.json --config $runtime.configPath
if ($LASTEXITCODE -ne 0) { throw 'QA build failed' }
node scripts/verify-windows-qa-runtime.mjs --manifest $runtime.manifestPath --config $runtime.configPath `
  --manifest-sha256 $runtime.manifestSha256 --expected-source $runtime.sourceSha `
  --executable src-tauri/target/x86_64-pc-windows-msvc/release/clippy-app.exe `
  --payload-root src-tauri/target/x86_64-pc-windows-msvc/release
```

本地仅编译时在 build 命令增加 `--no-bundle --no-sign`。显式 target 隔离默认 release 目录，
基础许可证资源继续合并；运行库和证明放在 QA EXE 的同目录。完整 `-RecordingQa` 门禁检查
暂存文件合同；构建后还须检查真实 payload。每次 QA 重建更新运行库，正式产品中央部署策略
保持；文件检查不证明安装或无运行库的 Windows 10/11 实际启动。

流程见 [`native-qa.md`](native-qa.md)。Windows 10/11 记录分别生成，不把模板记作通过：

```powershell
node scripts/manual-qa.mjs template --profile windows-11-x64 --sha <完整SHA> --version 0.1.20 --output <记录路径>
node scripts/manual-qa.mjs verify --input <记录路径> --output <报告路径>
```

重点包括混合 DPI/负坐标、跨屏选区与窗口速选、Pin/guide 定位、同权限和管理员目标粘贴、DACL、
系统凭据、默认/非默认音频设备、设备拔出、控制窗排除、暂停/恢复、强杀分段恢复和 30 分钟 A/V 漂移。
安装、升级、卸载、自启动和 updater 使用对应安装包单独记录。

“CI 成功”“本机编译成功”与“桌面验收通过”是独立结论，录屏 QA feature 不因此自动成为发布能力。
