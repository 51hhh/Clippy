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
 *  12. deb 不声明 libpipewire → 装上去的包在没装 PipeWire 的机器上根本起不来
 *      （硬链接失败发生在 main 之前，后端回退链一层都轮不到）
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
});

describe("deb 声明了二进制硬链接的库", () => {
  const deb = JSON.parse(read("src-tauri/tauri.conf.json")).bundle.linux.deb;

  /**
   * Tauri 的 deb 打包器只会自动写 webkit2gtk / gtk 那几条，不做 shlibdeps 扫描。
   * 而 PipeWire 取流（`screenshot/screencast.rs`）与 libwayshot 的 Wayland 截图让
   * 二进制**动态链接**上了 libpipewire / libgbm / libEGL —— 缺一个就是启动时
   * 动态链接失败，进程根本起不来，一层后端都轮不到。所以必须显式声明。
   *
   * t64 过渡把包名改了（24.04 起是 `libpipewire-0.3-0t64`），用 `|` 备选同时覆盖
   * 新旧发行版：写死单个名字会让 deb 在另一边直接装不上，比不声明更糟。
   */
  it.each([
    ["libpipewire-0.3", true],
    ["libgbm1", false],
    ["libegl1", false],
  ])("%s 在 depends 里", (library, needsAlternative) => {
    const entry = deb.depends.find((item) => item.includes(library));
    expect(entry, `depends 里没有 ${library}`).toBeTruthy();
    if (needsAlternative) expect(entry).toContain("|");
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

  it("updater 用的无后缀产物只从一个 label 上传一次", () => {
    // 无后缀名是更新器按固定 URL 找的那份；两个 runner 都传就会互相覆盖。
    const uploaders = [...release.matchAll(/if: matrix\.label == '([a-z0-9]+)'/g)].map((m) => m[1]);
    expect(new Set(uploaders).size).toBe(1);
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
    expect(deps).not.toMatch(/cropInPixels/);
    // 但选区本身还要喂给导出与选区翻译，别把它一起删了
    expect(source).toContain("cropInPixels");
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
