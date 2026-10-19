/** 设置页的本地 Clippy 归档交换。 */

function errorText(error) {
  return String(error?.message || error || "unknown error");
}

export function createArchiveTransfer({
  scopeSelect,
  sensitiveToggle,
  exportButton,
  importButton,
  status,
  exportArchive,
  importArchive,
  translate,
  onImported = () => {},
}) {
  let busy = false;

  function setBusy(next) {
    busy = next;
    scopeSelect.disabled = next;
    sensitiveToggle.disabled = next;
    exportButton.disabled = next;
    importButton.disabled = next;
    status.setAttribute("aria-busy", String(next));
  }

  function show(key, values = {}, kind = "") {
    status.textContent = translate(key, values);
    status.classList.toggle("error", kind === "error");
    status.hidden = false;
  }

  async function runExport() {
    if (busy) return;
    setBusy(true);
    show("settings.archive.exporting");
    try {
      const result = await exportArchive(scopeSelect.value, sensitiveToggle.checked);
      if (result === null) {
        status.hidden = true;
        return;
      }
      show("settings.archive.exported", result);
    } catch (error) {
      console.warn("导出 Clippy 归档失败:", error);
      show("settings.archive.exportFailed", { error: errorText(error) }, "error");
    } finally {
      setBusy(false);
    }
  }

  async function runImport() {
    if (busy) return;
    setBusy(true);
    show("settings.archive.importing");
    try {
      const result = await importArchive();
      if (result === null) {
        status.hidden = true;
        return;
      }
      show("settings.archive.imported", result);
      try {
        await onImported();
      } catch (error) {
        // 数据已经成功提交，刷新附属统计失败不能把结果改写成“导入失败”。
        console.warn("刷新归档导入后的设置状态失败:", error);
      }
    } catch (error) {
      console.warn("导入 Clippy 归档失败:", error);
      show("settings.archive.importFailed", { error: errorText(error) }, "error");
    } finally {
      setBusy(false);
    }
  }

  exportButton.addEventListener("click", () => { void runExport(); });
  importButton.addEventListener("click", () => { void runImport(); });

  return { runExport, runImport };
}
