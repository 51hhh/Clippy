import { describe, expect, it } from "vitest";
import {
  clipboardRowOffsets,
  clipboardVisibleRange,
  IMAGE_ROW_HEIGHT,
  TEXT_ROW_HEIGHT,
} from "../react/main/clipboardVirtualization.ts";

describe("clipboard list virtualization", () => {
  it("builds exact offsets for mixed text and image rows", () => {
    expect(clipboardRowOffsets([
      { content_type: "text" },
      { content_type: "image" },
      { content_type: "html" },
    ])).toEqual([
      0,
      TEXT_ROW_HEIGHT,
      TEXT_ROW_HEIGHT + IMAGE_ROW_HEIGHT,
      TEXT_ROW_HEIGHT * 2 + IMAGE_ROW_HEIGHT,
    ]);
  });

  it("keeps only a bounded window for ten thousand rows", () => {
    const offsets = clipboardRowOffsets(
      Array.from({ length: 10_000 }, () => ({ content_type: "text" })),
    );
    const range = clipboardVisibleRange(offsets, 385_000, 600);

    expect(range.start).toBeGreaterThan(4_900);
    expect(range.end - range.start).toBeLessThan(20);
    expect(range.paddingTop + (range.end - range.start) * TEXT_ROW_HEIGHT + range.paddingBottom)
      .toBe(offsets.at(-1));
  });

  it("clamps stale scroll positions after a result set shrinks", () => {
    const offsets = clipboardRowOffsets([
      { content_type: "image" },
      { content_type: "text" },
    ]);
    const range = clipboardVisibleRange(offsets, 1_000_000, 100, 0);

    expect(range).toMatchObject({ start: 0, end: 2, paddingTop: 0, paddingBottom: 0 });
  });
});
