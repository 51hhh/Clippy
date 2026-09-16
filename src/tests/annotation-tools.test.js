import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  annotationAt,
  annotationBounds,
  isEffectAnnotation,
  translateAnnotation,
} from "../react/annotation/annotationGeometry.ts";
import { drawAnnotation, renderExport } from "../react/annotation/canvasRenderer.ts";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../react/annotation/imageAdjustments.ts";
import { MANUAL_TOOLS, TOOL_DRAFTS, useCanvasInteractions } from "../react/annotation/useCanvasInteractions.ts";
import { TOOL_GROUPS } from "../react/capture-overlay/tools.tsx";

/**
 * 记录型 2D 上下文：jsdom 没有 canvas 后端，无法断言像素，
 * 因此这里断言绘制指令序列——工具之间的差异恰好体现在指令上。
 * 每条记录附带调用瞬间的画笔状态，save/restore 按真实语义压栈。
 */
function recordingContext() {
  const calls = [];
  const state = { globalAlpha: 1, fillStyle: "", strokeStyle: "", lineWidth: 1, filter: "none" };
  const stack = [];
  const target = {
    calls,
    save() {
      stack.push({ ...state });
      calls.push({ name: "save", args: [] });
    },
    restore() {
      Object.assign(state, stack.pop() ?? {});
      calls.push({ name: "restore", args: [] });
    },
  };
  return new Proxy(target, {
    get(_, prop) {
      if (prop in target) return target[prop];
      if (prop in state) return state[prop];
      return (...args) => {
        calls.push({
          name: String(prop),
          args,
          alpha: state.globalAlpha,
          lineWidth: state.lineWidth,
          fillStyle: state.fillStyle,
          filter: state.filter,
        });
      };
    },
    set(_, prop, value) {
      state[prop] = value;
      return true;
    },
    has: () => true,
  });
}

const named = (ctx, name) => ctx.calls.filter((call) => call.name === name);
const indexOfCall = (ctx, predicate) => ctx.calls.findIndex(predicate);
const fakeImage = { naturalWidth: 200, naturalHeight: 120 };

function vector(type, extra) {
  return { id: type, type, color: "#ff3b30", size: 4, ...extra };
}

describe("annotation geometry", () => {
  it("hits a hollow ellipse only near its outline", () => {
    const ellipse = vector("ellipse", { rect: { x: 0, y: 0, width: 100, height: 100 } });
    const behind = { id: "behind", type: "mosaic", rect: { x: 40, y: 40, width: 20, height: 20 } };
    // 圆心是空的：点在中间时必须落到底下的注解上，而不是被椭圆挡住。
    expect(annotationAt([behind, ellipse], { x: 50, y: 50 })?.id).toBe("behind");
    expect(annotationAt([behind, ellipse], { x: 0, y: 50 })?.id).toBe("ellipse");
  });

  it("gives the marker a wider hit reach than the pen", () => {
    const points = [
      { x: 0, y: 50 },
      { x: 100, y: 50 },
    ];
    const pen = vector("pen", { points });
    const marker = { ...vector("marker", { points }), id: "marker" };
    const off = { x: 50, y: 50 + 6.4 };
    expect(annotationAt([pen], off)).toBeNull();
    expect(annotationAt([marker], off)?.id).toBe("marker");
  });

  it("bounds line and measure annotations by their endpoints", () => {
    const box = { from: { x: 80, y: 10 }, to: { x: 20, y: 70 } };
    for (const type of ["line", "measure", "arrow"]) {
      expect(annotationBounds(vector(type, box))).toEqual({ x: 20, y: 10, width: 60, height: 60 });
    }
  });

  it("moves every annotation shape immutably", () => {
    const delta = { x: 5, y: -5 };
    const stroke = vector("marker", { points: [{ x: 1, y: 1 }] });
    expect(translateAnnotation(stroke, delta).points).toEqual([{ x: 6, y: -4 }]);
    expect(stroke.points).toEqual([{ x: 1, y: 1 }]);

    const segment = vector("measure", { from: { x: 0, y: 0 }, to: { x: 10, y: 10 } });
    expect(translateAnnotation(segment, delta)).toMatchObject({ from: { x: 5, y: -5 }, to: { x: 15, y: 5 } });

    for (const type of ["spotlight", "magnifier"]) {
      const effect = { id: type, type, rect: { x: 10, y: 10, width: 4, height: 4 } };
      expect(isEffectAnnotation(effect)).toBe(true);
      expect(translateAnnotation(effect, delta).rect).toEqual({ x: 15, y: 5, width: 4, height: 4 });
    }
  });
});

describe("annotation rendering", () => {
  const rect = { x: 10, y: 20, width: 40, height: 60 };

  it("fills the highlight rectangle translucently and strokes the plain rectangle", () => {
    const highlight = recordingContext();
    drawAnnotation(highlight, vector("highlight", { rect }));
    expect(named(highlight, "strokeRect")).toHaveLength(0);
    expect(named(highlight, "fillRect")[0].args).toEqual([10, 20, 40, 60]);
    expect(named(highlight, "fillRect")[0].alpha).toBeCloseTo(0.32);

    const plain = recordingContext();
    drawAnnotation(plain, vector("rect", { rect }));
    expect(named(plain, "fillRect")).toHaveLength(0);
    expect(named(plain, "strokeRect")[0].alpha).toBe(1);
  });

  it("draws the ellipse inscribed in its rectangle", () => {
    const ctx = recordingContext();
    drawAnnotation(ctx, vector("ellipse", { rect }));
    expect(named(ctx, "ellipse")[0].args.slice(0, 4)).toEqual([30, 50, 20, 30]);
    expect(named(ctx, "stroke")).toHaveLength(1);
    expect(named(ctx, "strokeRect")).toHaveLength(0);
  });

  it("draws the marker thicker and more transparent than the pen", () => {
    const points = [
      { x: 0, y: 0 },
      { x: 10, y: 10 },
    ];
    const pen = recordingContext();
    drawAnnotation(pen, vector("pen", { points }));
    const marker = recordingContext();
    drawAnnotation(marker, vector("marker", { points }));

    expect(named(pen, "stroke")[0].alpha).toBe(1);
    expect(named(marker, "stroke")[0].alpha).toBeCloseTo(0.32);
    expect(named(marker, "stroke")[0].lineWidth).toBeGreaterThan(named(pen, "stroke")[0].lineWidth);
  });

  it("labels the measure line with its length in source pixels", () => {
    const ctx = recordingContext();
    // 缩放到一半也要报告原图距离，否则截图里的数字会随预览缩放变化。
    drawAnnotation(ctx, vector("measure", { from: { x: 0, y: 0 }, to: { x: 120, y: 0 } }), 0.5);
    expect(named(ctx, "fillText")[0].args[0]).toBe("120 px");
    expect(named(ctx, "stroke")).toHaveLength(2);
  });

  it("draws an arrow head only for the arrow tool", () => {
    const segment = { from: { x: 0, y: 0 }, to: { x: 40, y: 0 } };
    const line = recordingContext();
    drawAnnotation(line, vector("line", segment));
    const arrow = recordingContext();
    drawAnnotation(arrow, vector("arrow", segment));

    expect(named(line, "stroke")).toHaveLength(1);
    expect(named(line, "fillText")).toHaveLength(0);
    expect(named(arrow, "stroke")).toHaveLength(2);
  });

  it("dims everything outside the spotlight with an even-odd fill", () => {
    const ctx = recordingContext();
    renderExport(
      ctx,
      fakeImage,
      { x: 0, y: 0, width: 200, height: 120 },
      [{ id: "s", type: "spotlight", rect: { x: 20, y: 20, width: 50, height: 40 } }],
      DEFAULT_IMAGE_ADJUSTMENTS,
    );
    const evenOdd = named(ctx, "fill").filter((call) => call.args[0] === "evenodd");
    expect(evenOdd).toHaveLength(1);
    expect(evenOdd[0].fillStyle).toBe("rgba(0, 0, 0, 0.55)");
    // 两个矩形（整幅图 + 选区）加上 even-odd 才能形成中间挖空的遮罩。
    expect(named(ctx, "rect").length).toBeGreaterThanOrEqual(3);
  });

  it("magnifies the source image inside an elliptical lens", () => {
    const ctx = recordingContext();
    renderExport(
      ctx,
      fakeImage,
      { x: 0, y: 0, width: 200, height: 120 },
      [{ id: "m", type: "magnifier", rect: { x: 20, y: 20, width: 60, height: 60 } }],
      DEFAULT_IMAGE_ADJUSTMENTS,
    );
    const zoomed = named(ctx, "drawImage").find((call) => call.args[3] === fakeImage.naturalWidth * 2);
    expect(zoomed).toBeTruthy();
    expect(zoomed.args[4]).toBe(fakeImage.naturalHeight * 2);
    expect(named(ctx, "clip").length).toBeGreaterThanOrEqual(2);
    expect(named(ctx, "ellipse").length).toBe(2);
  });

  it("draws effects before vector annotations regardless of insertion order", () => {
    const ctx = recordingContext();
    renderExport(
      ctx,
      fakeImage,
      { x: 0, y: 0, width: 200, height: 120 },
      [
        vector("pen", { points: [{ x: 0, y: 0 }, { x: 5, y: 5 }] }),
        { id: "b", type: "blur", rect: { x: 0, y: 0, width: 30, height: 30 } },
      ],
      DEFAULT_IMAGE_ADJUSTMENTS,
    );
    const blurred = indexOfCall(ctx, (call) => call.name === "drawImage" && call.filter.includes("blur("));
    const stroked = indexOfCall(ctx, (call) => call.name === "stroke");
    expect(blurred).toBeGreaterThanOrEqual(0);
    expect(blurred).toBeLessThan(stroked);
  });
});

describe("overlay tool wiring", () => {
  const toolbarTools = TOOL_GROUPS.flatMap((group) => group.tools.map((tool) => tool.id));

  /**
   * 覆盖层用 `select`（框选/移动/缩放选区）取代了标注核心的 `crop`：
   * 选区自己就是裁剪框，所以工具条里不该再出现 crop。
   */
  it("wires every toolbar tool to a draft shape or to manual handling", () => {
    expect(new Set(toolbarTools).size).toBe(toolbarTools.length);
    const wired = new Set([...Object.keys(TOOL_DRAFTS), ...MANUAL_TOOLS]);
    wired.delete("crop");
    wired.add("select");
    expect([...toolbarTools].sort()).toEqual([...wired].sort());
    expect(toolbarTools).not.toContain("crop");
  });

  // 分组是文档里那张表的真值来源（architecture.md#图片编辑器工具），改动必须同步
  it("keeps the documented group membership", () => {
    expect(
      TOOL_GROUPS.map((group) => [group.titleKey, group.tools.map((tool) => tool.id)]),
    ).toEqual([
      ["capture.toolGroup.select", ["select", "object", "eraser"]],
      [
        "capture.toolGroup.draw",
        ["pen", "marker", "rect", "ellipse", "line", "arrow", "measure", "text"],
      ],
      ["capture.toolGroup.effects", ["highlight", "blur", "mosaic", "spotlight", "magnifier"]],
    ]);
  });
});

describe("annotation interactions", () => {
  let container;
  let root;

  function mount(overrides = {}) {
    const api = {};
    const params = {
      imageRef: { current: { naturalWidth: 200, naturalHeight: 120 } },
      canvasRef: { current: { getBoundingClientRect: () => ({ left: 0, top: 0 }) } },
      scale: 1,
      tool: "pen",
      color: "#ff3b30",
      size: 4,
      text: "",
      annotations: [],
      selection: null,
      setSelection: vi.fn(),
      onSelect: vi.fn(),
      commitAnnotations: vi.fn(),
      ...overrides,
    };
    function Probe() {
      Object.assign(api, useCanvasInteractions(params));
      return null;
    }
    act(() => root.render(React.createElement(Probe)));
    return { api, params };
  }

  const pointer = (x, y) => ({ clientX: x, clientY: y, pointerId: 1, currentTarget: { setPointerCapture() {} } });

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    act(() => {
      root = createRoot(container);
    });
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it.each(Object.entries(TOOL_DRAFTS))("starts a %s draft of kind %s", (tool, kind) => {
    const { api } = mount({ tool });
    act(() => api.onPointerDown(pointer(12, 34)));
    expect(api.draft.kind).toBe(kind);
    expect(api.draft.annotation.type).toBe(tool);
    // 效果类注解没有颜色和线宽，画的是底图本身。
    expect(api.draft.annotation.color).toBe(kind === "effect" ? undefined : "#ff3b30");
  });

  it("commits a normalized rectangle when a shape is dragged backwards", () => {
    const { api, params } = mount({ tool: "ellipse" });
    act(() => api.onPointerDown(pointer(30, 40)));
    act(() => api.onPointerMove(pointer(10, 10)));
    act(() => api.onPointerUp());

    expect(params.commitAnnotations).toHaveBeenCalledTimes(1);
    const [committed] = params.commitAnnotations.mock.calls[0][0]([]);
    expect(committed).toMatchObject({ type: "ellipse", rect: { x: 10, y: 10, width: 20, height: 30 } });
    expect(api.draft).toBeNull();
  });

  it("erases one annotation per click and never on drag", () => {
    const annotations = [
      { id: "under", type: "rect", color: "#fff", size: 2, rect: { x: 0, y: 0, width: 60, height: 60 } },
      { id: "over", type: "mosaic", rect: { x: 10, y: 10, width: 20, height: 20 } },
    ];
    const { api, params } = mount({ tool: "eraser", annotations });

    act(() => api.onPointerDown(pointer(15, 15)));
    expect(params.commitAnnotations).toHaveBeenCalledTimes(1);
    expect(params.commitAnnotations.mock.calls[0][0](annotations).map((item) => item.id)).toEqual(["under"]);

    // 橡皮不建立草稿，所以拖动不会连删——一次点击对应一条撤销记录。
    act(() => api.onPointerMove(pointer(16, 16)));
    act(() => api.onPointerUp());
    expect(params.commitAnnotations).toHaveBeenCalledTimes(1);
  });

  it("ignores an eraser click on empty canvas", () => {
    const { api, params } = mount({ tool: "eraser", annotations: [] });
    act(() => api.onPointerDown(pointer(150, 100)));
    expect(params.commitAnnotations).not.toHaveBeenCalled();
    expect(params.onSelect).not.toHaveBeenCalled();
  });
});
