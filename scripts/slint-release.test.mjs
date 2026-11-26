import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { buildQaBuildInfo } from "./qa-build-info.mjs";
import { installVersions } from "./slint-release.mjs";

test("native versions keep alpha, beta, RC and stable in the same upgrade order", () => {
  const tags = ["v0.2.0-alpha.1", "v0.2.0-beta.1", "v0.2.0-rc.1", "v0.2.0", "v0.2.1-alpha.1"];
  const versions = tags.map(installVersions);
  assert.deepEqual(versions.map((value) => value.deb),
    ["0.2.0~alpha.1", "0.2.0~beta.1", "0.2.0~rc.1", "0.2.0", "0.2.1~alpha.1"]);
  assert.deepEqual(versions.map((value) => value.windows), ["0.2.1", "0.2.30", "0.2.60", "0.2.99", "0.2.101"]);
  assert.deepEqual(versions.map((value) => value.macosBuild), versions.map((value) => value.windows));
  assert.throws(() => installVersions("v0.2.0-beta.31"), /out of range/);
});

test("Slint QA workflows pass valid playback fields to the metadata builder", () => {
  const workflow = readFileSync(new URL("../.github/workflows/native-qa.yml", import.meta.url), "utf8");
  for (const [platform, player] of [["windows-x64", "wasapi"], ["macos-x64", "coreaudio"]]) {
    const option = workflow.match(new RegExp(`--field (recording[-_]playback=${player})(?=\\s|$)`));
    assert.ok(option, `missing QA playback field for ${platform}`);
    const report = buildQaBuildInfo({
      backend: "slint", platform, signing: "test", sha: "a".repeat(40), fields: [option[1]],
    });
    assert.ok(report.split("\n").includes(`recording_playback=${player}`));
  }
});
