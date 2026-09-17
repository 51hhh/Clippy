// @vitest-environment node
import { describe, expect, it } from "vitest";
import {
  loadFrontendSources,
  validateFrontendBoundaries,
} from "../../scripts/check-html-sinks.mjs";

const repositoryRoot = new URL("../..", import.meta.url).pathname;

function sources() {
  return loadFrontendSources(repositoryRoot);
}

describe("frontend HTML and Tauri boundaries", () => {
  it("accepts the checked-in frontend boundary", () => {
    expect(validateFrontendBoundaries(sources())).toEqual([]);
  });

  it("rejects a raw innerHTML write", () => {
    const input = sources();
    input.set("src/js/preview/new-renderer.js", "content.innerHTML = userValue;\n");
    expect(validateFrontendBoundaries(input)).toContain(
      "src/js/preview/new-renderer.js:1 未登记或未清洗的 innerHTML 写入",
    );
  });

  it("rejects a sanitizer bypass in an allowlisted sink", () => {
    const input = sources();
    input.set(
      "src/js/preview/content-renderers.js",
      input.get("src/js/preview/content-renderers.js")
        .replace("getLibraries().DOMPurify.sanitize(rawHtml, PURIFY_CONFIG)", "rawHtml"),
    );
    expect(validateFrontendBoundaries(input)).toEqual(
      expect.arrayContaining([expect.stringContaining("未登记或未清洗的 innerHTML 写入")]),
    );
  });

  it("rejects a Tauri import outside the registered API domain modules", () => {
    const input = sources();
    input.set("src/js/preview/new-renderer.js", 'import "@tauri-apps/api/core";\n');
    expect(validateFrontendBoundaries(input)).toContain(
      "src/js/preview/new-renderer.js:1 只有已登记的 src/js/api 领域模块可导入 @tauri-apps",
    );
  });

  it("rejects a new unregistered module inside the API directory", () => {
    const input = sources();
    input.set("src/js/api/accidental.ts", 'import "@tauri-apps/api/core";\n');
    expect(validateFrontendBoundaries(input)).toContain(
      "src/js/api/accidental.ts:1 只有已登记的 src/js/api 领域模块可导入 @tauri-apps",
    );
  });
});
