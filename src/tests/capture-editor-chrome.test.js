import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as i18n from "../i18n/i18n.js";
import { EditorFooter } from "../react/capture/EditorChrome.tsx";

let container;
let root;

function render(props) {
  act(() => {
    root.render(React.createElement(EditorFooter, props));
  });
}

function buttonLabeled(label) {
  return [...container.querySelectorAll("button")].find((button) =>
    button.textContent.includes(label),
  );
}

beforeEach(() => {
  i18n.init("en");
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

describe("capture editor footer", () => {
  const base = {
    busy: false,
    canUndo: false,
    canRedo: false,
    canExport: true,
    onUndo: () => {},
    onRedo: () => {},
    onReset: () => {},
  };

  it("keeps Save and Save As as separate export actions", () => {
    const onExport = vi.fn();
    render({ ...base, onExport });

    act(() => buttonLabeled("Save As...").click());
    // 直接保存与另存为必须是两个动作，否则用户永远拿不到对话框。
    expect(onExport).toHaveBeenLastCalledWith("saveAs");

    act(() => buttonLabeled("Save").click());
    expect(onExport).toHaveBeenLastCalledWith("save");
    expect(onExport).toHaveBeenCalledTimes(2);
  });

  it("disables both save actions without an exportable selection", () => {
    render({ ...base, canExport: false, onExport: vi.fn() });

    expect(buttonLabeled("Save As...").disabled).toBe(true);
    expect(buttonLabeled("Save").disabled).toBe(true);
  });
});
