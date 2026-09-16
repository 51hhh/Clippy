import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as search from "../js/search.js";

function setup() {
  document.body.innerHTML = '<input id="search" />';
  const input = document.getElementById("search");
  const calls = [];
  search.init(input, (value) => calls.push(value));
  return { input, calls };
}

describe("search", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.body.innerHTML = "";
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("debounce 后触发 trim 后的查询", () => {
    const { input, calls } = setup();

    input.value = "  apple  ";
    input.dispatchEvent(new Event("input"));

    expect(calls).toEqual([]);
    vi.advanceTimersByTime(199);
    expect(calls).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(calls).toEqual(["apple"]);
  });

  it("连续输入只触发最后一次查询", () => {
    const { input, calls } = setup();

    input.value = "a";
    input.dispatchEvent(new Event("input"));
    vi.advanceTimersByTime(100);
    input.value = "ap";
    input.dispatchEvent(new Event("input"));
    vi.advanceTimersByTime(200);

    expect(calls).toEqual(["ap"]);
  });

  it("clear 立即清空并触发空查询", () => {
    const { input, calls } = setup();

    input.value = "apple";
    search.clear();

    expect(input.value).toBe("");
    expect(calls).toEqual([""]);
  });
});
