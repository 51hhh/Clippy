import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as telemetry from "../js/telemetry.js";

describe("telemetry", () => {
  beforeEach(() => {
    telemetry.disable();
  });

  afterEach(() => {
    telemetry.disable();
  });

  it("默认禁用：emit 不缓存、不通知订阅者", () => {
    const seen = [];
    telemetry.subscribe((r) => seen.push(r));
    telemetry.emit("noop", { x: 1 });
    expect(telemetry.snapshot()).toEqual([]);
    expect(seen).toEqual([]);
    expect(telemetry.isEnabled()).toBe(false);
  });

  it("enable 后缓存并通知订阅者，cap 在 bufferLimit", () => {
    telemetry.enable({ bufferLimit: 2 });
    const seen = [];
    telemetry.subscribe((r) => seen.push(r.event));

    telemetry.emit("a");
    telemetry.emit("b");
    telemetry.emit("c");

    expect(seen).toEqual(["a", "b", "c"]);
    const buf = telemetry.snapshot().map((r) => r.event);
    expect(buf).toEqual(["b", "c"]);
  });

  it("subscribe 返回值能取消订阅", () => {
    telemetry.enable({ bufferLimit: 5 });
    const seen = [];
    const unsub = telemetry.subscribe((r) => seen.push(r.event));
    telemetry.emit("first");
    unsub();
    telemetry.emit("second");
    expect(seen).toEqual(["first"]);
  });

  it("disable 后清空缓冲与订阅者", () => {
    telemetry.enable({ bufferLimit: 5 });
    telemetry.subscribe(() => {});
    telemetry.emit("a");
    telemetry.disable();
    expect(telemetry.snapshot()).toEqual([]);
    expect(telemetry.isEnabled()).toBe(false);
  });
});
