import { describe, expect, it } from "vitest";
import { parseInitialPinProject } from "../react/pin/projectSchema.ts";

const EFFECT = { blurRadius: 8, mosaicCell: 12, spotlightDim: 0.55, magnifierZoom: 2 };
const ADJUSTMENTS = { grayscale: false, brightness: 0, contrast: 0, saturation: 0, cornerRadius: 0 };

function project(annotations = [], overrides = {}) {
  return {
    format: "clippy-pin-project",
    formatVersion: 2,
    rendererVersion: 1,
    source: { width: 320, height: 180, sha256: "a".repeat(64) },
    document: { annotations, adjustments: ADJUSTMENTS },
    ...overrides,
  };
}

describe("pin project runtime schema", () => {
  it("hydrates a v2 document and normalizes renderer appearance parameters", () => {
    const parsed = parseInitialPinProject(project([
      { id: "pen-1", type: "pen", color: "#fff", size: 3, points: [{ x: 1, y: 2 }, { x: 3, y: 4 }] },
      { id: "blur-1", type: "blur", rect: { x: 10, y: 20, width: 30, height: 40 }, effect: EFFECT },
      { id: "text-1", type: "text", color: "#fff", size: 4, at: { x: 8, y: 9 }, text: "safe", fontFamily: "system-ui" },
    ]));

    expect(parsed).toMatchObject({ rendererVersion: 1, sourceWidth: 320, sourceHeight: 180 });
    expect(parsed.annotations[1].effect).toEqual(EFFECT);
    expect(parsed.annotations[2].fontFamily).toBe("system-ui");
  });

  it.each([
    ["future renderer", project([], { rendererVersion: 2 })],
    ["invalid hash", { ...project(), source: { width: 320, height: 180, sha256: "bad" } }],
    ["non-finite coordinate", project([{ id: "x", type: "pen", color: "#fff", size: 2, points: [{ x: NaN, y: 0 }] }])],
    ["out-of-bounds rect", project([{ id: "x", type: "blur", rect: { x: 300, y: 0, width: 30, height: 20 }, effect: EFFECT }])],
    ["duplicate id", project([
      { id: "x", type: "text", color: "#fff", size: 2, at: { x: 1, y: 1 }, text: "a" },
      { id: "x", type: "text", color: "#fff", size: 2, at: { x: 2, y: 2 }, text: "b" },
    ])],
    ["invalid effect", project([{ id: "x", type: "blur", rect: { x: 0, y: 0, width: 3, height: 3 }, effect: { ...EFFECT, blurRadius: Infinity } }])],
    ["overlong text", project([{ id: "x", type: "text", color: "#fff", size: 2, at: { x: 1, y: 1 }, text: "x".repeat(16 * 1024 + 1) }])],
  ])("rejects %s before it reaches React state", (_name, value) => {
    expect(parseInitialPinProject(value)).toBeNull();
  });
});
