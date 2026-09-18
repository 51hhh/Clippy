import { describe, expect, it } from "vitest";
import { parseGuideRect } from "../react/longshot-guide/geometry";

describe("longshot input-transparent guide geometry", () => {
  it("keeps fractional mixed-DPI crop geometry", () => {
    expect(parseGuideRect("?x=1&y=2&width=30.5&height=40.25", 100, 80)).toEqual({
      x: 1,
      y: 2,
      width: 30.5,
      height: 40.25,
    });
  });

  it("rejects non-finite, empty, negative and off-monitor rectangles", () => {
    for (const search of [
      "?x=NaN&y=2&width=30&height=40",
      "?y=2&width=30&height=40",
      "?x=-1&y=2&width=30&height=40",
      "?x=1&y=2&width=0&height=40",
      "?x=80&y=2&width=30&height=40",
      "?x=1&y=60&width=30&height=40",
    ]) {
      expect(parseGuideRect(search, 100, 80), search).toBeNull();
    }
  });
});
