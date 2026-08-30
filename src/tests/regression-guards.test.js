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
 */
import { execFileSync } from "node:child_process";
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

  // CLI 把钩子的 cwd 设成仓库根还是 src-tauri/ 取决于版本与调用方式（cargo tauri
  // 与 npx tauri 就不一致）。写死 `cd ../src` 时，从仓库根跑会解析到仓库外面去。
  it.each([
    ["beforeDevCommand", conf.build.beforeDevCommand],
    ["beforeBuildCommand", conf.build.beforeBuildCommand],
  ])("%s 从任一 cwd 都能进到前端目录", (_name, command) => {
    const cd = command.split("&&")[0];
    for (const cwd of [repoRoot, resolve(repoRoot, "src-tauri")]) {
      const landed = execFileSync("sh", ["-c", `${cd} && pwd`], { cwd, encoding: "utf8" }).trim();
      expect(landed, cwd).toBe(frontendRoot);
    }
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
