import { describe, expect, it } from "vitest";
import { annotationAt, translateAnnotation } from "../react/annotation/annotationGeometry.ts";
import { commitHistory, redoHistory, undoHistory } from "../react/annotation/useHistory.ts";

describe("annotation document model", () => {
  it("selects the topmost annotation and moves it immutably", () => {
    const lower = { id: "lower", type: "rect", color: "red", size: 2, rect: { x: 0, y: 0, width: 40, height: 40 } };
    const upper = { id: "upper", type: "mosaic", rect: { x: 10, y: 10, width: 20, height: 20 } };
    expect(annotationAt([lower, upper], { x: 15, y: 15 })?.id).toBe("upper");
    expect(translateAnnotation(upper, { x: 5, y: -2 }).rect).toEqual({ x: 15, y: 8, width: 20, height: 20 });
    expect(upper.rect).toEqual({ x: 10, y: 10, width: 20, height: 20 });
  });

  it("supports undo, redo, and clears redo after a new commit", () => {
    const initial = { past: [], present: "a", future: [] };
    const second = commitHistory(initial, "b");
    expect(undoHistory(second)).toEqual({ past: [], present: "a", future: ["b"] });
    expect(redoHistory(undoHistory(second)).present).toBe("b");
    expect(commitHistory(undoHistory(second), "c").future).toEqual([]);
  });
});
