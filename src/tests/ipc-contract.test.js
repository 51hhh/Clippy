// @vitest-environment node
import { describe, expect, it } from "vitest";
import { fileURLToPath } from "node:url";
import {
  loadContractSources,
  validateContract,
} from "../../scripts/check-ipc-contract.mjs";

const repositoryRoot = fileURLToPath(new URL("../..", import.meta.url));

function sources(lineEnding) {
  const input = loadContractSources(repositoryRoot);
  // 每种宿主都验证两种检出格式；格式转换只用于合同夹具，不改真实源码或校验规则。
  const convert = (source) => source.replace(/\r\n/g, "\n").replace(/\n/g, lineEnding);
  return {
    rustSources: new Map([...input.rustSources].map(([path, source]) => [path, convert(source)])),
    libSource: convert(input.libSource),
    apiSource: convert(input.apiSource),
    accessSource: convert(input.accessSource),
  };
}

describe.each([
  { name: "LF", lineEnding: "\n" },
  { name: "CRLF", lineEnding: "\r\n" },
])("IPC contract gate ($name)", ({ lineEnding }) => {
  it("accepts the checked-in Rust, frontend and window access contract", () => {
    expect(validateContract(sources(lineEnding))).toEqual([]);
  });

  it("fails when a command definition is removed from generate_handler", () => {
    const input = sources(lineEnding);
    const changed = input.libSource.replace(/^[ \t]*commands::get_clips,\r?\n/m, "");
    expect(changed).not.toBe(input.libSource);
    input.libSource = changed;
    expect(validateContract(input)).toContain(
      "Rust command 未注册到 generate_handler!: get_clips",
    );
  });

  it("fails when an api wrapper points at a misspelled command", () => {
    const input = sources(lineEnding);
    input.apiSource = input.apiSource.replace('"get_clips"', '"get_clipz"');
    expect(validateContract(input)).toContain(
      "前端 API invoke 指向未注册命令: get_clipz",
    );
  });

  it("fails when a viewer command is omitted from its window allowlist", () => {
    const input = sources(lineEnding);
    const changed = input.accessSource.replace(/^[ \t]*"get_viewer_payload",\r?\n/m, "");
    expect(changed).not.toBe(input.accessSource);
    input.accessSource = changed;
    expect(validateContract(input)).toContain(
      "图片查看器前端命令未加入 IMAGE_VIEWER_COMMANDS: get_viewer_payload",
    );
  });
});
