import { describe, expect, it } from "vitest";

import {
  DETECT_SAMPLE_CHARS,
  MAX_RENDER_CHARS,
  detectionSample,
  limitForRender,
} from "../js/preview/large-text.js";

describe("预览面板对超大条目的两条闸门", () => {
  it("短条目一个字符都不动，行为和加闸门之前完全一样", () => {
    const text = "const answer = 42;\n";
    expect(detectionSample(text)).toBe(text);
    expect(limitForRender(text)).toEqual({ body: text, truncated: false, omitted: 0 });
  });

  it("正好卡在上限上不算截断（边界要能重复判定，别每次少一个字符）", () => {
    const exact = "x".repeat(MAX_RENDER_CHARS);
    const result = limitForRender(exact);
    expect(result.truncated).toBe(false);
    expect(result.body).toHaveLength(MAX_RENDER_CHARS);

    const sample = "y".repeat(DETECT_SAMPLE_CHARS);
    expect(detectionSample(sample)).toHaveLength(DETECT_SAMPLE_CHARS);
  });

  it("语言检测只看开头：highlightAuto 是按注册的每种语法各跑一遍全文的", () => {
    const huge = "z".repeat(DETECT_SAMPLE_CHARS * 4);
    expect(detectionSample(huge)).toHaveLength(DETECT_SAMPLE_CHARS);
  });

  it("超限时说清少了多少，别让用户以为内容就这么长", () => {
    const huge = "w".repeat(MAX_RENDER_CHARS + 1234);
    const result = limitForRender(huge);
    expect(result.truncated).toBe(true);
    expect(result.body).toHaveLength(MAX_RENDER_CHARS);
    expect(result.omitted).toBe(1234);
  });

  it("拿到 null/undefined 不能崩：预览面板对空条目也会走这条路", () => {
    expect(detectionSample(null)).toBe("");
    expect(detectionSample(undefined)).toBe("");
    expect(limitForRender(null)).toEqual({ body: "", truncated: false, omitted: 0 });
  });
});
