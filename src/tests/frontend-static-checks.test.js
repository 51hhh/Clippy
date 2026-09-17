// @vitest-environment node
import { describe, expect, it } from "vitest";
import ts from "typescript";
import { findFloatingPromises } from "../scripts/check-floating-promises.mjs";

function programFor(source) {
  const fileName = "/clippy-static-fixture.js";
  const options = {
    allowJs: true,
    checkJs: true,
    noEmit: true,
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
  };
  const host = ts.createCompilerHost(options);
  const originalGetSourceFile = host.getSourceFile.bind(host);
  host.fileExists = path => path === fileName || ts.sys.fileExists(path);
  host.readFile = path => path === fileName ? source : ts.sys.readFile(path);
  host.getSourceFile = (path, languageVersion, onError, shouldCreateNewSourceFile) => {
    if (path === fileName) return ts.createSourceFile(path, source, languageVersion, true, ts.ScriptKind.JS);
    return originalGetSourceFile(path, languageVersion, onError, shouldCreateNewSourceFile);
  };
  return ts.createProgram([fileName], options, host);
}

describe("vanilla JS Promise gate", () => {
  it("rejects an unhandled async call", () => {
    const program = programFor("async function work() {}\nwork();\n");
    expect(findFloatingPromises(program, file => file.fileName.endsWith("clippy-static-fixture.js")))
      .toEqual([expect.stringContaining("未处理 Promise")]);
  });

  it("accepts explicit void and catch handling", () => {
    const program = programFor(`
      async function work() {}
      void work();
      work().catch(() => {});
    `);
    expect(findFloatingPromises(program, file => file.fileName.endsWith("clippy-static-fixture.js")))
      .toEqual([]);
  });
});
