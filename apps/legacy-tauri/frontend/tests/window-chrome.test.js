import { afterEach, describe, expect, it, vi } from "vitest";
import { bindWindowChrome } from "../js/window-chrome.js";

function flushActions() {
  return new Promise((resolve) => queueMicrotask(resolve));
}

function fixture() {
  document.body.innerHTML = `
    <header id="titlebar"><h1>Settings</h1><button id="inside">Inside</button></header>
    <button id="minimize">Minimize</button>
    <button id="maximize">Maximize</button>
    <button id="close">Close</button>
  `;
  const actions = {
    startDragging: vi.fn(async () => {}),
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    requestClose: vi.fn(async () => {}),
  };
  const dispose = bindWindowChrome({
    titlebar: document.getElementById("titlebar"),
    minimizeButton: document.getElementById("minimize"),
    maximizeButton: document.getElementById("maximize"),
    closeButton: document.getElementById("close"),
    ...actions,
  });
  return { actions, dispose };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("vanilla window chrome", () => {
  it("drags only from non-interactive titlebar content", async () => {
    const { actions } = fixture();
    document.querySelector("#titlebar h1").dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
    await flushActions();
    expect(actions.startDragging).toHaveBeenCalledOnce();

    document.getElementById("inside").dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
    await flushActions();
    expect(actions.startDragging).toHaveBeenCalledOnce();
  });

  it("routes double-click and all controls through injected window actions", async () => {
    const { actions } = fixture();
    document.querySelector("#titlebar h1").dispatchEvent(new MouseEvent("dblclick", { bubbles: true, button: 0 }));
    document.getElementById("minimize").click();
    document.getElementById("maximize").click();
    document.getElementById("close").click();
    await flushActions();

    expect(actions.toggleMaximize).toHaveBeenCalledTimes(2);
    expect(actions.minimize).toHaveBeenCalledOnce();
    expect(actions.requestClose).toHaveBeenCalledOnce();
  });

  it("removes every listener when disposed", async () => {
    const { actions, dispose } = fixture();
    dispose();
    document.querySelector("#titlebar h1").dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, button: 0 }));
    document.getElementById("close").click();
    await flushActions();
    expect(actions.startDragging).not.toHaveBeenCalled();
    expect(actions.requestClose).not.toHaveBeenCalled();
  });
});
