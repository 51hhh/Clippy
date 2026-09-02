/**
 * regression-guards.test.js — 已经踩过一次的坑
 *
 * 这里的每一条都对应一个真实报障，且都是"改错了不会立刻报错、只会在界面上
 * 悄悄退化"的那类问题，所以用结构断言钉住，而不是靠人再点一遍：
 *   1. tauri.conf.json 的 before*Command 依赖固定 cwd → `can't cd to ../src`，dev 起不来
 *   2. `#codec-output` 改回 <pre> → 多字段结果的按钮行装不进去（<pre> 只容纳短语内容）
 *   3. 列表行重新显示内容类型 → 主栏 HTML、侧栏 YAML 的自相矛盾又回来
 *   4. preview-panel.js 自己再嗅探一遍内容类型 → 判定重新分叉成两套标准
 *   5. release notes 的下载链接与构建矩阵的发行版标签不同步 → 发布页上是死链
 *   6. 发布脚本假定 cargo-tauri 存在 → 只在真正打 tag 时才炸（runner 上只有 npm 侧 CLI）
 *   7. prependClip 按索引 +1 挪焦点 → 面板关着时到达的新条目把焦点挤到第二行，
 *      按 Pin 贴出的是上一张图（用户报障的正是这条）
 *   8. 全局 Pin 退回前端列表缓存 / 剪贴板写入不唤醒 watcher → 同一个症状的另外两条放大器
 *   8b. 全局 Pin 信任"面板没焦点时留下的焦点行" → 侧栏开着时列表不释放，焦点跟着老条目
 *       挪到第 1 行，截完图按 Pin 贴出的还是上一张（第 7 条修完仍然复现的就是这条）
 *   9. 选区压暗搬回 drawScene → 拖一次选区就是几十次全屏重采样，帧率掉下来但功能"是对的"
 *  10. 列表行收整个 snapshot / 回调在渲染里新建 → memo 失效，一次按键重渲全部 30 行
 *  11. 列表行取原图画 48 px 缩略图 → 每开一次面板十几 MB IPC + 十几次全尺寸 PNG 解码
 *  12. 默认构建移除 PipeWire 后 deb 仍声明它 → Ubuntu 22 被迫安装无用运行库，
 *      也掩盖了依赖图是否意外重新启用 PipeWire 的真实回归
 *  13. release 在 latest.json 上传前就取消 draft → 用户可见的更新端点短暂缺文件
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const frontendRoot = resolve(process.cwd());
const repoRoot = resolve(frontendRoot, "..");

function read(relativeToRepo) {
  return readFileSync(resolve(repoRoot, relativeToRepo), "utf8");
}

describe("tauri 构建钩子不依赖 cwd", () => {
  const conf = JSON.parse(read("src-tauri/tauri.conf.json"));

  // 让 Tauri 自己按配置文件目录解析 cwd；脚本不再包含 POSIX shell 的 cd、重定向和
  // 逻辑运算符，因此同一份配置可以由 sh、cmd.exe 或 PowerShell 启动。
  it.each([
    ["beforeDevCommand", conf.build.beforeDevCommand, "npm run dev"],
    ["beforeBuildCommand", conf.build.beforeBuildCommand, "npm run build"],
  ])("%s 使用跨平台结构化 cwd", (_name, command, expectedScript) => {
    expect(command).toEqual({ script: expectedScript, cwd: "../src" });
    expect(resolve(repoRoot, "src-tauri", command.cwd)).toBe(frontendRoot);
    expect(command.script).not.toMatch(/[;&|<>]/);
  });
});

describe("Linux CI 固守 Ubuntu 22 构建基线", () => {
  const buildWorkflow = read(".github/workflows/build.yml");
  const qaWorkflow = read(".github/workflows/native-qa.yml");
  const releaseWorkflow = read(".github/workflows/release.yml");

  it.each([
    ["CI", buildWorkflow],
    ["release", releaseWorkflow],
  ])("%s 使用 Jammy 且不安装 PipeWire 开发包", (_name, workflow) => {
    expect(workflow).toContain("ubuntu-22.04");
    expect(workflow).not.toContain("ubuntu-24.04");
    expect(workflow).not.toContain("libpipewire-0.3-dev");
  });

  it("默认依赖图出现 pipewire-rs 时 CI 会失败", () => {
    expect(buildWorkflow).toContain("Default Linux dependency graph must not include pipewire-rs");
    expect(releaseWorkflow).toContain("Default Linux dependency graph must not include pipewire-rs");
  });

  it("Rust target 缓存按 runner 隔离", () => {
    // Linux runner 只看 runner.os 会把 Jammy 与更新发行版都归到 Linux，缓存中的
    // build-script 会因较新 glibc 无法在 Ubuntu 22 启动；runner 名必须进入 key。
    const cacheBlocks = [...buildWorkflow.matchAll(/- name: Rust cache[\s\S]*?workspaces: src-tauri/g)];
    expect(cacheBlocks).toHaveLength(2);
    for (const [block] of cacheBlocks) {
      expect(block).toContain("prefix-key: v1-rust");
      expect(block).toMatch(/key: (ubuntu-22\.04|\$\{\{ matrix\.runner \}\})/);
    }
  });

  it("Jammy 构建的 AppImage 在 Ubuntu 24 执行强制运行 smoke", () => {
    expect(qaWorkflow).toMatch(/build-linux:[\s\S]*runs-on: ubuntu-22\.04/);
    expect(qaWorkflow).toMatch(
      /smoke-ubuntu24:[\s\S]*needs: build-linux[\s\S]*runs-on: ubuntu-24\.04/,
    );
    expect(qaWorkflow).toContain('CLIPPY_APPIMAGE_SMOKE_REQUIRED: "1"');
    expect(qaWorkflow).toContain("./scripts/smoke-appimage-x11.sh");
    expect(qaWorkflow).toContain("appimage-x11-smoke.*/runner.log");
    expect(qaWorkflow).not.toMatch(/^\s+path: src-tauri\/target\/appimage-x11-smoke\.\*$/m);
  });

  it("AppImage smoke 不允许子进程从 bash -s 的脚本流读取 stdin", () => {
    const smokeScript = read("scripts/smoke-appimage-x11.sh");
    expect(smokeScript).toContain('"$APP_RUN" </dev/null');
    expect(smokeScript).toContain("ffmpeg -nostdin");
    expect(smokeScript).toContain('</dev/null >"$FFMPEG_LOG"');
  });
});

describe("原生平台由真实 runner 编译", () => {
  const buildWorkflow = read(".github/workflows/build.yml");

  it.each(["windows-latest", "macos-latest"])("%s 执行 check 与 clippy", (runner) => {
    expect(buildWorkflow).toContain(runner);
  });
  it("原生 job 不用 Linux 交叉工具链冒充验证", () => {
    expect(buildWorkflow).toMatch(/native-check:[\s\S]*cargo check --all-targets/);
    expect(buildWorkflow).toMatch(/native-check:[\s\S]*cargo clippy --all-targets -- -D warnings/);
  });

  it("原生 job 执行 Rust 测试，平台无关前端门禁只在 Jammy 执行一次", () => {
    const nativeJob = buildWorkflow.slice(buildWorkflow.indexOf("  native-check:"));
    expect(nativeJob).toContain("cargo test");
    expect(nativeJob).not.toContain("npx vitest run");
    expect(nativeJob).not.toContain("npx tsc --noEmit");
    expect(nativeJob).not.toContain("npx vite build");
    expect(buildWorkflow.slice(0, buildWorkflow.indexOf("  native-check:"))).toContain(
      "npx vitest run",
    );
  });

  it("手动 QA workflow 为同一 SHA 生成四架构包、Ubuntu 24 证据与九环境记录", () => {
    const qaWorkflow = read(".github/workflows/native-qa.yml");
    expect(buildWorkflow).not.toContain("QA artifact");
    expect(qaWorkflow).toMatch(/on:\s*\n\s+workflow_dispatch:/);
    for (const artifact of [
      "qa-linux-x64-${{ github.sha }}",
      "qa-windows-x64-${{ github.sha }}",
      "qa-macos-${{ matrix.asset_arch }}-${{ github.sha }}",
      "qa-ubuntu24-x11-smoke-${{ github.sha }}",
      "qa-record-templates-${{ github.sha }}",
    ]) {
      expect(qaWorkflow).toContain(artifact);
    }
    expect(qaWorkflow.match(/uses: actions\/upload-artifact@v7/g)).toHaveLength(5);
    expect(qaWorkflow.match(/retention-days: 14/g)).toHaveLength(5);
    expect(qaWorkflow).toContain("signing=unsigned-qa-only");
    expect(qaWorkflow).toContain("signing=ad-hoc-qa-only");
    expect(qaWorkflow).toContain("./scripts/finalize-appimage.sh");
    expect(qaWorkflow).toContain("rust_target: x86_64-apple-darwin");
    expect(qaWorkflow).toContain("rust_target: aarch64-apple-darwin");
    expect(qaWorkflow).toContain("linux-gnome-wayland-ubuntu24");
    expect(qaWorkflow).toContain("--config src-tauri/tauri.linux.conf.json");
    expect(qaWorkflow).toContain("--config src-tauri/tauri.windows.conf.json");
    expect(qaWorkflow).toContain("--config src-tauri/tauri.macos.conf.json");
    expect(qaWorkflow).toContain("--config src-tauri/tauri.macos.qa.conf.json");
    expect(qaWorkflow).not.toMatch(/macOS QA[\s\S]*--no-sign/);
    expect(qaWorkflow).toContain("plutil -extract CFBundleExecutable raw");
    expect(qaWorkflow).toContain('lipo -archs "$EXECUTABLE"');
    expect(qaWorkflow).not.toContain('Contents/MacOS/Clippy"');
  });
});

describe("deb 只声明默认二进制真实需要的额外运行库", () => {
  const deb = JSON.parse(read("src-tauri/tauri.linux.conf.json")).bundle.linux.deb;

  /**
   * Tauri 的 deb 打包器只会自动写 webkit2gtk / gtk 那几条，不做 shlibdeps 扫描。
   * 默认构建已经不编译 PipeWire 增强后端，ELF 的 NEEDED 表里也没有 PipeWire/libspa；
   * deb 若继续声明它，会把可选能力错误变成安装期硬依赖。libwayshot 与 WebKit 的
   * Wayland 链仍使用 GBM/EGL，所以保留这两项兼容声明。
   */
  it.each(["libgbm1", "libegl1"])("%s 在 depends 里", (library) => {
    const entry = deb.depends.find((item) => item.includes(library));
    expect(entry, `depends 里没有 ${library}`).toBeTruthy();
  });

  it("PipeWire 保持为源码构建时显式启用的可选能力", () => {
    expect(deb.depends.some((item) => item.includes("pipewire"))).toBe(false);
  });
});

describe("Tauri 打包目标按平台隔离", () => {
  const base = JSON.parse(read("src-tauri/tauri.conf.json"));
  const linux = JSON.parse(read("src-tauri/tauri.linux.conf.json"));
  const windows = JSON.parse(read("src-tauri/tauri.windows.conf.json"));
  const macos = JSON.parse(read("src-tauri/tauri.macos.conf.json"));
  const macosQa = JSON.parse(read("src-tauri/tauri.macos.qa.conf.json"));

  it("公共配置不携带任何平台专属 bundle", () => {
    expect(base.bundle.targets).toBeUndefined();
    expect(base.bundle.linux).toBeUndefined();
    expect(base.bundle.windows).toBeUndefined();
    expect(base.bundle.macOS).toBeUndefined();
  });

  it.each([
    ["Linux", linux.bundle.targets, ["deb", "appimage"]],
    ["Windows", windows.bundle.targets, ["nsis", "msi"]],
    ["macOS", macos.bundle.targets, ["app", "dmg"]],
  ])("%s 只构建自己的安装包", (_name, actual, expected) => {
    expect(actual).toEqual(expected);
  });

  it("Windows 安装器会补装缺失的 WebView2", () => {
    expect(windows.bundle.windows.webviewInstallMode.type).toBe("downloadBootstrapper");
  });

  it("macOS 最低版本与应用身份保持稳定", () => {
    expect(macos.bundle.macOS.minimumSystemVersion).toBe("11.0");
    // Tauri 会警告公共标识 com.clippy.app 与 macOS 的 .app 包扩展冲突；macOS 因此
    // 使用固定覆盖值。这里不能只断言“不以 .app 结尾”：任意改名都会让 TCC 授权、
    // WebView 数据目录与更新身份一起漂移。
    expect(macos.identifier).toBe("com.clippy.desktop");
    expect(base.identifier).toBe("com.clippy.app");
    expect(linux.identifier).toBeUndefined();
    expect(windows.identifier).toBeUndefined();
  });

  it("macOS Pin 同时启用透明 WebView 的配置与 Cargo feature", () => {
    // Tauri 构建脚本会把 Cargo feature 与公共配置一起校验；放到平台覆盖或 target
    // dependency 都会在 cargo check 阶段被判定为不匹配。
    expect(base.app.macOSPrivateApi).toBe(true);
    expect(read("src-tauri/Cargo.toml")).toMatch(
      /^tauri\s*=\s*\{[^\n]*features\s*=\s*\[[^\n]*"macos-private-api"/m,
    );
  });

  it("macOS QA 使用真实 ad-hoc 签名而不是伪标记无签名包", () => {
    expect(macosQa.bundle.macOS.signingIdentity).toBe("-");
  });
});

describe("codec 输出区能装下多字段结果", () => {
  it("#codec-output 是 <div> 而不是 <pre>", () => {
    const document = new DOMParser().parseFromString(read("src/index.html"), "text/html");
    const output = document.getElementById("codec-output");
    expect(output?.tagName.toLowerCase()).toBe("div");
    expect(output?.classList.contains("codec-output")).toBe(true);
  });
});

describe("release 下载链接与构建矩阵同步", () => {
  // 产物名是 `Clippy_<ver>_amd64_<label>.deb`，label 来自构建矩阵。删矩阵条目却忘了
  // 删 release notes 里的那一行，发布页就会挂一条 404 死链（反过来则是漏传产物）。
  const release = read(".github/workflows/release.yml");

  it("下载表里的发行版后缀恰好等于矩阵里的 label", () => {
    const labels = [...release.matchAll(/^\s*- runner: \S+\s*\n\s*label: (\S+)\s*$/gm)].map(
      (m) => m[1],
    );
    const linked = [...release.matchAll(/Clippy_\$\{VER\}_amd64_([a-z0-9]+)\./g)].map((m) => m[1]);
    expect(labels.length).toBeGreaterThan(0);
    expect([...new Set(linked)].sort()).toEqual([...new Set(labels)].sort());
  });

  it("AppImage 签名不写死 cargo-tauri", () => {
    // release runner 上只有 tauri-action 自带的 CLI 与 `src/` 锁定的 npm CLI，没有
    // cargo-tauri；写死 `cargo tauri signer sign` 只会在真正打 tag 的时候才 no such command。
    const script = read("scripts/finalize-appimage.sh");
    expect(script).not.toMatch(/cargo tauri signer/);
    expect(script).toMatch(/tauri_cli signer sign/);
    expect(script).toContain("src/node_modules/.bin/tauri");
    // 签名后必须验产物存在，否则 updater 会静默拿不到 .sig
    expect(script).toMatch(/! -s "\$\{TARGET\}\.sig"/);
  });

  it("updater 用的无后缀 Linux 产物只从单一 Jammy label 进入汇总", () => {
    // 无后缀名是更新器按固定 URL 找的那份；Linux matrix 若扩成多个 runner，必须先
    // 重新设计 artifact 名，不能让多个生产者写入同一个 release-linux-x64。
    const linuxJob = release.slice(
      release.indexOf("  build-linux:"),
      release.indexOf("  build-windows:"),
    );
    expect([...linuxJob.matchAll(/^\s+label: ([a-z0-9]+)$/gm)].map((match) => match[1])).toEqual([
      "ubuntu22",
    ]);
    expect(linuxJob.match(/dist-artifacts\/Clippy_\$\{VER\}_amd64\.AppImage"/g)).toHaveLength(1);
  });

  it("Windows 与双架构 macOS 产物交给受测生成器写入 manifest", () => {
    expect(release).toContain("windows-latest");
    expect(release).toContain("macos-latest");
    expect(release).toContain("Clippy_$env:APP_VERSION`_x64-setup.exe.sig");
    expect(release).toContain("Clippy_${APP_VERSION}_${ASSET_ARCH}.app.tar.gz.sig");
    expect(release).toContain("node scripts/generate-updater-manifest.mjs");
    expect(release).toContain("--signature-dir release-assets");
    expect(release).toContain('--platforms "$PLATFORMS"');
  });

  it("latest.json 上传成功后才公开发布", () => {
    const uploadManifest = release.indexOf('gh release upload "${TAG}" latest.json');
    const publishRelease = release.indexOf('--draft=false');
    expect(uploadManifest).toBeGreaterThan(0);
    expect(publishRelease).toBeGreaterThan(uploadManifest);
  });

  it("release 默认只读，只有 artifact 生产者和最终发布者提升最小权限", () => {
    expect(release).toMatch(/^permissions:\s*\n\s+contents: read/m);
    expect(release.match(/artifact-metadata: write/g)).toHaveLength(3);
    expect(release.match(/artifact-metadata: read/g)).toHaveLength(1);
    expect(release.match(/contents: write/g)).toHaveLength(1);
    expect(release).not.toContain("write-all");
  });

  it("正式构建前核对同 SHA 原生 CI 并解析签名策略", () => {
    expect(release).toContain("node scripts/verify-native-ci.mjs");
    expect(release).toContain('--sha "$GITHUB_SHA"');
    expect(release).toContain("Missing required updater secret");
    expect(release).toContain("windows_signing_mode=self-signed");
    expect(release).toContain("Build ad-hoc signed Tauri");
    expect(release).toContain("--config src-tauri/tauri.macos.qa.conf.json");
  });

  it("可用平台先汇总为 workflow artifacts，再创建并公开 release", () => {
    const linuxJob = release.slice(
      release.indexOf("  build-linux:"),
      release.indexOf("  build-windows:"),
    );
    expect(linuxJob).toContain("name: release-linux-x64");
    expect(linuxJob).not.toContain("gh release");
    const createDraft = release.indexOf("Ensure unpublished draft release exists");
    const uploadNative = release.indexOf("Upload native artifacts to release");
    const publishRelease = release.indexOf("Publish complete release");
    expect(createDraft).toBeGreaterThan(0);
    expect(uploadNative).toBeGreaterThan(createDraft);
    expect(publishRelease).toBeGreaterThan(uploadNative);
  });

  it("Windows 无 PFX 时生成自签名证书并在发布说明中明确告警", () => {
    expect(release).toContain("New-SelfSignedCertificate");
    expect(release).toContain('Subject = "CN=Clippy Self-Signed Release"');
    expect(release).toContain('Provider = "Microsoft Software Key Storage Provider"');
    expect(release).toContain('KeyUsage = "DigitalSignature"');
    expect(release).toContain('"2.5.29.37={text}1.3.6.1.5.5.7.3.3"');
    expect(release).toContain('@("Valid", "NotTrusted")');
    expect(release).not.toContain("WINDOWS_TRUSTED_ROOT_THUMBPRINT");
    expect(release).not.toContain("Cert:\\CurrentUser\\Root");
    expect(release).toContain("Windows SmartScreen");
  });

  it("macOS 两个架构始终使用 ad-hoc 签名进入构建与 manifest", () => {
    expect(release).toContain("rust_target: aarch64-apple-darwin");
    expect(release).toContain("rust_target: x86_64-apple-darwin");
    expect(release).toContain("Signature=adhoc");
    expect(release).toContain("plutil -extract CFBundleExecutable raw");
    expect(release).not.toContain('Contents/MacOS/Clippy"');
    expect(release).toContain("未经过 Apple Developer ID 公证");
    expect(release).toContain(
      'PLATFORMS="linux-x86_64,windows-x86_64,darwin-aarch64,darwin-x86_64"',
    );
  });

  it("发布产物 Actions 使用 Node 24 运行时版本", () => {
    expect(release.match(/actions\/upload-artifact@v7/g)).toHaveLength(3);
    expect(release.match(/actions\/download-artifact@v8/g)).toHaveLength(1);
    expect(release).not.toMatch(/actions\/(?:upload|download)-artifact@v[1-6]\b/);
  });
});

describe("内容类型只有一套标准", () => {
  // 后端 content_type 只有 text/html/image，预览按内容嗅探出 YAML/JWT/TIMESTAMP…
  // 两边同时显示必然自相矛盾。类型只归预览面板的 badge。
  it("展示格式化模块不提供类型格式化函数", () => {
    const source = read("src/js/clipboard/formatters.js");
    expect(source).not.toMatch(/export function formatType/);
  });

  it("两个列表行渲染器都不写类型标签", () => {
    // js/clipboard/row-renderer.js 目前在运行时不生效（app.js 不传 listEl，实际渲染
    // 走 react/main/ClipboardRow.tsx）。正因为它是哑的，改动只落在它上面时用户看不到
    // 效果——上次"删掉主栏类型显示"就是这么丢的，所以两份一起钉。
    for (const path of ["src/js/clipboard/row-renderer.js", "src/react/main/ClipboardRow.tsx"]) {
      const source = read(path);
      expect(source, path).not.toContain("formatType");
      expect(source, path).not.toContain("clip-row-html-badge");
      expect(source, path).not.toMatch(/clipboard\.type\./);
    }
  });

  it("预览面板不自己再嗅探一遍类型", () => {
    const source = read("src/js/preview-panel.js");
    expect(source).toContain("classifyText");
    // 同步可判的检测器只允许出现在 classify.js 的表里；面板里再调一次就是第二套标准
    for (const detector of ["identifyHash", "detectEncoding", "isTimestamp", "isUuid", "isColor"]) {
      expect(source, detector).not.toMatch(new RegExp(`\\b${detector}\\s*\\(`));
    }
  });
});

describe("Pin 贴的是当前焦点条目，不是上一条", () => {
  // 报障："截屏后 pin 图会显示之前的图片"。真正的原因在前端焦点索引：面板关闭时
  // `releaseMemory` 把列表清空，随后到达的 `clip-added` 让 prependClip 把 focusedRow +1,
  // 于是重新打开面板时焦点停在**第二行**，而 Pin 的两个入口（全局快捷键与 Ctrl+P）
  // 都读 getFocusedClip()。列表行上没有 Pin 按钮，所以这条路是唯一的。
  it("两份 prependClip 都按 id 跟踪焦点，不做索引加减", () => {
    // 两份实现必须同步：js/clipboard-list.js 目前运行时不生效（见上面的 row-renderer 注释），
    // 只改 React 那一份的话，下次谁把渲染切回去，这个 bug 就原地复活。
    for (const path of ["src/js/clipboard-list.js", "src/react/main/clipboardStore.ts"]) {
      const code = read(path)
        .split("\n")
        .filter((line) => !/^\s*(\/\/|\*|\/\*)/.test(line))
        .join("\n");
      expect(code, path).not.toMatch(/focusedRow\s*[+-]\s*1/);
      expect(code, path).toMatch(/previousFocus/);
    }
  });

  it("列表内容被释放后不报告焦点行", () => {
    // focusedRow 归 0 而不是 -1 就是上面那条 +1 的燃料：0 在空列表上是个不存在的行。
    const source = read("src/js/clipboard/navigation-state.js");
    const release = source.slice(source.indexOf("export function releaseNavigation"));
    expect(release).toMatch(/focusedRow:\s*-1/);
  });

  it("app.js 的 Pin 快捷键路径不调用 getLatestClip", () => {
    const source = read("src/js/app.js");
    expect(source).not.toContain("getLatestClip");
    expect(source).toContain("resolvePinTarget");
  });

  it("全局 Pin 把面板是否有焦点一起传给 resolvePinTarget", () => {
    // 少了这个参数，焦点行的残影又会被当成用户意图：侧栏开着时失焦不隐藏窗口，
    // 列表与焦点活过整个截图流程，prependClip 按 id 把焦点跟着老条目挪到第 1 行。
    const source = read("src/js/app.js");
    const call = source.slice(source.indexOf("resolvePinTarget("));
    expect(call.slice(0, call.indexOf(");"))).toContain("document.hasFocus()");
  });

  it("重新聚焦面板时两条分支都复位焦点", () => {
    // `refresh()` 只做钳位（normalizeAfterRefresh），不复位；只在"不脏"的分支里
    // restoreRender 等于"面板关着期间来了新条目"时焦点留在老条目上。
    const source = read("src/js/app.js");
    const body = source.slice(
      source.indexOf("async function onWindowFocus"),
      source.indexOf("function onWindowBlur"),
    );
    expect(body).toMatch(/isDirty\(\)\)\s*await clipboardList\.refresh\(\);/);
    expect([...body.matchAll(/restoreRender\(\)/g)]).toHaveLength(1);
    expect(body).not.toMatch(/else/);
  });

  it("剪贴板写入口写完都会唤醒 watcher", () => {
    // 少敲一次的后果是那条写入路径重新变成"最多 500 ms 后才进历史"。
    const source = read("src-tauri/src/clipboard_watcher/writer.rs");
    const writes = [...source.matchAll(/^pub fn clipboard_set_\w+/gm)].length;
    expect(writes).toBe(3);
    expect([...source.matchAll(/wake::nudge\(\);/g)]).toHaveLength(writes);
  });

  it("watcher 的轮询等待全部走唤醒口，没有裸 sleep", () => {
    // 任何一处漏改都会让那条分支继续睡满 500 ms，唤醒对它无效。
    const source = read("src-tauri/src/clipboard_watcher.rs");
    expect(source).not.toMatch(/thread::sleep/);
    expect(source).toMatch(/wake::wait_for_next_poll\(POLL_INTERVAL\)/);
  });
});

describe("拖动选区不引发画布重绘", () => {
  // drawScene 每帧要把整张冻结帧（2560×1600）以 high 质量缩绘进 1920×1200 的画布。
  // 压暗和虚线蓝框放在画布上时，选区是它的输入，于是拖动/缩放选区的每个 pointermove
  // 都白做一次全图重采样。搬到 CSS 之后这条最高频的交互一次都不碰画布。
  it("drawScene 不接收裁剪矩形，也不画压暗", () => {
    const source = read("src/react/annotation/canvasRenderer.ts");
    const start = source.indexOf("export function drawScene");
    const signature = source.slice(start, source.indexOf("{", start));
    expect(signature).not.toMatch(/crop/);
    expect(source).not.toContain("drawCropOverlay");
  });

  it("覆盖层的重绘 effect 不依赖选区", () => {
    const source = read("src/react/capture-overlay/App.tsx");
    const call = source.indexOf("drawScene(");
    expect(call).toBeGreaterThan(0);
    const deps = source.slice(call, source.indexOf("]);", call));
    expect(deps).not.toMatch(/selection/);
    // 选区只进后端提交/翻译；禁止恢复 WebView 最终 PNG 导出。
    expect(source).toContain("{ ...selection, sessionId:");
    expect(source).toContain("rendererVersion: 2");
    expect(source).not.toContain("exportPngBase64");
  });

  it("压暗与虚线框由 .selection 这一层 CSS 承担", () => {
    const rule = read("src/react/capture-overlay/overlay.css")
      .split("\n")
      .find((line) => line.startsWith(".selection {"));
    expect(rule).toMatch(/box-shadow:[^;]*vmax/);
    expect(rule).toMatch(/outline:[^;]*dashed/);
  });
});

describe("列表行只取缩略图，不取原图", () => {
  // 库里存的是原图（一张全屏截图 2560×1600 / 几 MB），行里那格是 48 CSS px。
  // 退回 getClipImage 后功能照样对，只是每开一次面板就把十几 MB 送进 webview
  // 并做十几次全尺寸 PNG 解码，全落在 webview 那一个线程上。
  it("两个列表行渲染器都走 getClipThumbnail", () => {
    for (const path of ["src/js/clipboard-list.js", "src/react/main/ClipboardRow.tsx"]) {
      const code = read(path);
      expect(code, path).toContain("getClipThumbnail");
      expect(code.replace(/^\s*(\/\/|\*|\/\*).*$/gm, ""), path).not.toMatch(/getClipImage\s*[,(]/);
    }
  });

  it("预览面板仍然取原图", () => {
    // 预览是全尺寸显示，缩略图会糊；这条钉住"别顺手把预览也改过去"。
    expect(read("src/js/preview/content-renderers.js")).toMatch(/getClipImage\(clip\.id\)/);
  });

  it("后端缩略图命令已注册且带上限", () => {
    expect(read("src-tauri/src/lib.rs")).toContain("commands::get_clip_thumbnail");
    expect(read("src-tauri/src/commands/clipboard.rs")).toMatch(/THUMBNAIL_MAX_EDGE:\s*u32\s*=/);
  });
});

describe("移动焦点不重渲整份列表", () => {
  // 每次导航都会产生一份新 snapshot。行如果直接收 snapshot，一次按键就要把 30 行
  // 连着每行 5 个 lucide 图标全部 reconcile 一遍；拍成标量 + memo 之后只重渲两行。
  // 这两条都是"改回去功能照样对、只是变慢"，所以只能结构断言。
  it("ClipboardRow 被 memo 包着，且不接收整个 snapshot", () => {
    const source = read("src/react/main/ClipboardRow.tsx");
    expect(source).toMatch(/export const ClipboardRow = memo\(/);
    expect(source).not.toMatch(/snapshot[?.:]/);
    expect(source).not.toContain("ClipboardSnapshot");
  });

  it("行的回调表是模块级常量，不在渲染里新建", () => {
    // 每次渲染新建一份回调 = props 每次都变 = memo 完全失效（而且看不出来）。
    const source = read("src/react/main/ClipboardWorkspace.tsx");
    expect(source).toMatch(/^const ROW_HANDLERS: ClipboardRowHandlers = \{/m);
    const row = source.slice(source.indexOf("<ClipboardRow"), source.indexOf("/>", source.indexOf("<ClipboardRow")));
    expect(row).toContain("handlers={ROW_HANDLERS}");
    expect(row).not.toMatch(/=>/);
  });
});

describe("截图诊断不泄露窗口身份", () => {
  it("窗口探测只打印数量、状态与几何", () => {
    const source = read("src-tauri/src/screenshot/backends.rs");
    const start = source.indexOf("fn window_probe_diagnostics()");
    expect(start).toBeGreaterThan(0);
    const diagnostic = source.slice(start);
    expect(diagnostic).toContain("window.is_minimized()");
    expect(diagnostic).not.toContain("window.title()");
    expect(diagnostic).not.toContain("window.pid()");
  });
});
