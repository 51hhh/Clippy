import { strict as assert } from "node:assert";
import { test } from "node:test";
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
