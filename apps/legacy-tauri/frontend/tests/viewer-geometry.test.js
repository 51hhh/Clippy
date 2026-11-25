import { describe, expect, it } from "vitest";
import { boundedDpr, fitView, imageOrigin, sourcePoint, zoomAt } from "../react/viewer/geometry";
import { ViewerRequests } from "../react/viewer/requests";

describe("immutable viewer coordinates and requests", () => {
  const size = { width: 800, height: 600 }, source = { width: 4000, height: 3000 };
  it("keeps the same source pixel under a wheel anchor after arbitrary pan and zoom", () => {
    const before = { scale: .5, x: 37, y: -91, mode: "custom" };
    const point = { x: 253, y: 402 };
    const original = sourcePoint(point, before, size, source);
    const after = zoomAt(before, 1.7, point, size, source);
    expect(sourcePoint(point, after, size, source)).toEqual(original);
  });
  it("preserves the zoom anchor even when the image is several viewports away", () => {
    const before = { scale: .5, x: 8000, y: -6000, mode: "custom" };
    const point = { x: 253, y: 402 }, origin = imageOrigin(before, size, source);
    const after = zoomAt(before, 1.7, point, size, source);
    const next = imageOrigin(after, size, source);
    expect((point.x - next.x) / after.scale).toBeCloseTo((point.x - origin.x) / before.scale);
    expect((point.y - next.y) / after.scale).toBeCloseTo((point.y - origin.y) / before.scale);
    expect(Math.abs(after.x)).toBeGreaterThan(8000);
    expect(sourcePoint(point, after, size, source)).toBeNull();
  });
  it("rejects padding for sampling while annotation drag clamps to source boundaries", () => {
    const view = fitView(size, source), origin = imageOrigin(view, size, source);
    expect(sourcePoint({ x: origin.x - 1, y: origin.y }, view, size, source)).toBeNull();
    expect(sourcePoint({ x: origin.x - 100, y: origin.y + 90000 }, view, size, source, true)).toEqual({ x: 0, y: 3000 });
  });
  it("keeps backing stores bounded independently of source size or zoom", () => {
    const area = { width: 8192, height: 8192 }, dpr = boundedDpr(4, area);
    expect(area.width * area.height * dpr * dpr).toBeLessThanOrEqual(16_777_216);
    expect(area.width * dpr).toBeLessThanOrEqual(8192);
    expect(boundedDpr(4, { width: 800, height: 600 })).toBe(4);
  });
  it("fits a short canvas without subtracting a fixed toolbar height from all remaining space", () => {
    const view = fitView({ width: 280, height: 90 }, source);
    expect(view.scale * source.height).toBeCloseTo(54);
    expect(imageOrigin(view, { width: 280, height: 90 }, source).y).toBeGreaterThanOrEqual(0);
  });
  it("isolates channels and rejects stale identities after dispose and StrictMode reactivation", () => {
    const authority = new ViewerRequests({ sessionId: "A", snapshotId: "A1" });
    const first = authority.begin("ocr"), scan = authority.begin("scan");
    expect(authority.accepts("ocr", first, { ...first, value: "ok" })).toBe(true);
    authority.begin("ocr");
    expect(authority.accepts("ocr", first)).toBe(false);
    expect(authority.accepts("scan", scan, { ...scan, snapshotId: "B1", value: "wrong" })).toBe(false);
    authority.dispose(); authority.activate();
    expect(authority.accepts("scan", scan)).toBe(false);
    expect(authority.begin("scan").requestId).toBeGreaterThan(scan.requestId);
  });
});
