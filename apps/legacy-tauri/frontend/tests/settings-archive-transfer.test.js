import { beforeEach, describe, expect, it, vi } from "vitest";
import { createArchiveTransfer } from "../js/settings/archive-transfer.js";

function setup(overrides = {}) {
  document.body.innerHTML = `
    <select id="scope"><option value="full">full</option><option value="favorites">favorites</option></select>
    <input id="sensitive" type="checkbox">
    <button id="export">export</button>
    <button id="import">import</button>
    <p id="status" hidden></p>`;
  const elements = {
    scopeSelect: document.querySelector("#scope"),
    sensitiveToggle: document.querySelector("#sensitive"),
    exportButton: document.querySelector("#export"),
    importButton: document.querySelector("#import"),
    status: document.querySelector("#status"),
  };
  const deps = {
    exportArchive: vi.fn().mockResolvedValue(null),
    importArchive: vi.fn().mockResolvedValue(null),
    translate: (key, values = {}) => `${key}:${JSON.stringify(values)}`,
    onImported: vi.fn(),
    ...overrides,
  };
  const controller = createArchiveTransfer({ ...elements, ...deps });
  return { ...elements, ...deps, controller };
}

describe("settings archive transfer", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("passes scope and explicit sensitive opt-in to export", async () => {
    const exportArchive = vi.fn().mockResolvedValue({
      path: "/tmp/history.clippy.zip",
      clips: 3,
      groups: 1,
      workspaces: 2,
    });
    const view = setup({ exportArchive });
    view.scopeSelect.value = "favorites";
    view.sensitiveToggle.checked = true;

    await view.controller.runExport();

    expect(exportArchive).toHaveBeenCalledWith("favorites", true);
    expect(view.status.textContent).toContain("settings.archive.exported");
    expect(view.exportButton.disabled).toBe(false);
  });

  it("keeps cancellation quiet and refreshes stats after successful import", async () => {
    const onImported = vi.fn();
    const view = setup({ onImported });
    await view.controller.runImport();
    expect(view.status.hidden).toBe(true);
    expect(onImported).not.toHaveBeenCalled();

    const result = { clipsAdded: 2, clipsMerged: 1, groupsAdded: 1, workspacesAdded: 4 };
    view.importArchive.mockResolvedValueOnce(result);
    await view.controller.runImport();

    expect(onImported).toHaveBeenCalledOnce();
    expect(view.status.textContent).toContain("settings.archive.imported");
  });

  it("restores controls after failure so the operation can be retried", async () => {
    const exportArchive = vi.fn()
      .mockRejectedValueOnce(new Error("broken archive"))
      .mockResolvedValueOnce({ path: "/tmp/retry.clippy.zip", clips: 0, groups: 0, workspaces: 0 });
    const view = setup({ exportArchive });

    await view.controller.runExport();
    expect(view.status.classList.contains("error")).toBe(true);
    expect(view.exportButton.disabled).toBe(false);
    expect(view.scopeSelect.disabled).toBe(false);

    await view.controller.runExport();
    expect(view.status.classList.contains("error")).toBe(false);
    expect(exportArchive).toHaveBeenCalledTimes(2);
  });

  it("does not report a committed import as failed when stats refresh fails", async () => {
    const result = { clipsAdded: 1, clipsMerged: 0, groupsAdded: 0, workspacesAdded: 0 };
    const view = setup({
      importArchive: vi.fn().mockResolvedValue(result),
      onImported: vi.fn().mockRejectedValue(new Error("stats unavailable")),
    });

    await view.controller.runImport();

    expect(view.status.textContent).toContain("settings.archive.imported");
    expect(view.status.classList.contains("error")).toBe(false);
  });
});
