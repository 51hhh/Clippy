import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  App,
  type PinWorkspaceLibraryServices,
} from "../react/pin-workspace-library/App.tsx";
import type { PinWorkspaceLibrarySnapshot } from "../js/ipc-types.ts";

const snapshot: PinWorkspaceLibrarySnapshot = {
  groups: [{ id: 3, name: "Research", sortOrder: 0 }],
  items: [
    {
      id: 11,
      groupId: null,
      contentType: "text",
      previewText: "Pinned notes",
      contentWidth: 320,
      contentHeight: 180,
      scale: 1,
      opacity: 1,
      locked: false,
      above: false,
      updatedAt: 1_700_000_000,
      open: false,
    },
    {
      id: 12,
      groupId: 3,
      contentType: "image",
      previewText: null,
      contentWidth: 640,
      contentHeight: 480,
      scale: 0.75,
      opacity: 0.8,
      locked: true,
      above: true,
      updatedAt: 1_700_000_100,
      open: true,
    },
  ],
};

describe("pin workspace library app", () => {
  let root: Root;
  let services: PinWorkspaceLibraryServices;
  const reactEnvironment = globalThis as typeof globalThis & {
    IS_REACT_ACT_ENVIRONMENT?: boolean;
  };

  beforeEach(() => {
    reactEnvironment.IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="root"></div>';
    services = {
      ready: vi.fn(async () => {}),
      list: vi.fn(async () => structuredClone(snapshot)),
      thumbnail: vi.fn(async () => "cG5n"),
      show: vi.fn(async () => false),
      assignGroup: vi.fn(async () => {}),
      remove: vi.fn(async () => {}),
      createGroup: vi.fn(async () => ({ id: 4, name: "Ideas", sortOrder: 1 })),
      renameGroup: vi.fn(async () => true),
      deleteGroup: vi.fn(async () => true),
      startDrag: vi.fn(async () => {}),
      close: vi.fn(async () => {}),
      changed: vi.fn(async () => () => {}),
    };
    root = createRoot(document.getElementById("root")!);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    delete reactEnvironment.IS_REACT_ACT_ENVIRONMENT;
  });

  async function render() {
    await act(async () => root.render(<App services={services} />));
    await act(async () => Promise.resolve());
  }

  function button(text: string): HTMLButtonElement {
    return [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((candidate) => candidate.textContent?.trim() === text)!;
  }

  function setInputValue(input: HTMLInputElement, value: string) {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }

  it("loads lightweight cards and requests image thumbnails lazily", async () => {
    await render();
    expect(services.ready).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("Pinned notes");
    expect(document.body.textContent).toContain("640 × 480");
    expect(services.thumbnail).toHaveBeenCalledWith(12);
    expect(document.querySelector<HTMLImageElement>(".workspace-image-preview img")?.src)
      .toBe("data:image/png;base64,cG5n");
  });

  it("drags from title text while keeping the close control interactive", async () => {
    await render();
    const heading = document.querySelector<HTMLHeadingElement>(".pin-library-titlebar h1")!;
    await act(async () => heading.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
    expect(services.startDrag).toHaveBeenCalledOnce();

    await act(async () => document.querySelector<HTMLButtonElement>(".window-close")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
    expect(services.startDrag).toHaveBeenCalledOnce();
  });

  it("refreshes open state when Pin windows change outside the library", async () => {
    let notify: (() => void) | undefined;
    vi.mocked(services.changed).mockImplementation(async (callback) => {
      notify = callback;
      return () => {};
    });
    await render();
    vi.mocked(services.list).mockResolvedValueOnce({ groups: snapshot.groups, items: [] });
    await act(async () => notify?.());
    await act(async () => Promise.resolve());
    expect(services.list).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("No saved pins here");
  });

  it("filters groups and routes show and assignment through workspace ids", async () => {
    await render();
    await act(async () => button("Research1").click());
    expect(document.body.textContent).not.toContain("Pinned notes");

    await act(async () => button("Focus").click());
    expect(services.show).toHaveBeenCalledWith(12);

    const select = document.querySelector<HTMLSelectElement>('.workspace-card select')!;
    await act(async () => {
      select.value = "";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(services.assignGroup).toHaveBeenCalledWith(12, null);
  });

  it("uses inline confirmation and explains that an open Pin stays visible", async () => {
    await render();
    await act(async () => button("Research1").click());
    const remove = document.querySelector<HTMLButtonElement>('button[aria-label="Remove"]')!;
    await act(async () => remove.click());
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("stay on screen");
    expect(services.remove).not.toHaveBeenCalled();

    await act(async () => button("Remove").click());
    expect(services.remove).toHaveBeenCalledWith(12);
    expect(document.body.textContent).toContain("No saved pins here");
  });

  it("creates, renames, and deletes groups without exposing storage paths", async () => {
    await render();
    const inputs = [...document.querySelectorAll<HTMLInputElement>("aside input")];
    await act(async () => {
      setInputValue(inputs[0], "Ideas");
    });
    await act(async () => document.querySelector<HTMLButtonElement>('.group-create button')!.click());
    expect(services.createGroup).toHaveBeenCalledWith("Ideas");
    expect([...document.querySelectorAll("nav button")]
      .filter((candidate) => candidate.textContent === "Ideas0")).toHaveLength(1);

    const rename = [...document.querySelectorAll<HTMLInputElement>("aside input")].at(-1)!;
    await act(async () => {
      setInputValue(rename, "Ideas 2");
    });
    await act(async () => button("Rename").click());
    expect(services.renameGroup).toHaveBeenCalledWith(4, "Ideas 2");

    await act(async () => button("Delete").click());
    expect(services.deleteGroup).not.toHaveBeenCalled();
    const confirmDelete = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .filter((candidate) => candidate.textContent?.trim() === "Delete").at(-1)!;
    await act(async () => confirmDelete.click());
    expect(services.deleteGroup).toHaveBeenCalledWith(4);
  });
});
