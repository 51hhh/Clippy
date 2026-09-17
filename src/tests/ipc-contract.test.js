// @vitest-environment node
import { describe, expect, it } from "vitest";
import {
  loadContractSources,
  validateContract,
} from "../../scripts/check-ipc-contract.mjs";

const repositoryRoot = new URL("../..", import.meta.url).pathname;

function sources() {
  return loadContractSources(repositoryRoot);
}

describe("IPC contract gate", () => {
  it("accepts the checked-in Rust, frontend and window access contract", () => {
    expect(validateContract(sources())).toEqual([]);
  });

  it("fails when a command definition is removed from generate_handler", () => {
    const input = sources();
    input.libSource = input.libSource.replace("            commands::get_clips,\n", "");
    expect(validateContract(input)).toContain(
      "Rust command 未注册到 generate_handler!: get_clips",
    );
  });

  it("fails when an api wrapper points at a misspelled command", () => {
    const input = sources();
    input.apiSource = input.apiSource.replace('"get_clips"', '"get_clipz"');
    expect(validateContract(input)).toContain(
      "前端 API invoke 指向未注册命令: get_clipz",
    );
  });

  it("fails when a viewer command is omitted from its window allowlist", () => {
    const input = sources();
    input.accessSource = input.accessSource.replace('    "get_viewer_payload",\n', "");
    expect(validateContract(input)).toContain(
      "图片查看器前端命令未加入 IMAGE_VIEWER_COMMANDS: get_viewer_payload",
    );
  });
});
