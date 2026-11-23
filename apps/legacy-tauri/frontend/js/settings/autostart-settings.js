/** 开发版与正式版共享启动项名称；未确定运行身份前不可修改。 */
export function createAutostartSettings({ toggle, isDevBinary, isEnabled, enable, disable, translate, notify }) {
  toggle.disabled = true;

  toggle.addEventListener("change", async () => {
    if (toggle.disabled) return;
    const enabled = toggle.checked;
    toggle.disabled = true;
    try {
      await (enabled ? enable() : disable());
    } catch (error) {
      toggle.checked = !enabled;
      notify(translate("settings.saveFailed", { error }));
    } finally {
      toggle.disabled = false;
    }
  });

  return {
    async load() {
      toggle.disabled = true;
      try {
        if (await isDevBinary()) {
          toggle.checked = false;
          const row = toggle.closest(".setting-toggle-row");
          if (row && !row.parentElement?.querySelector(".autostart-dev-hint")) {
            const hint = document.createElement("p");
            hint.className = "setting-hint autostart-dev-hint";
            hint.dataset.i18n = "settings.autostart.devHint";
            hint.textContent = translate(hint.dataset.i18n);
            row.insertAdjacentElement("afterend", hint);
          }
          return;
        }
        toggle.checked = await isEnabled();
        toggle.disabled = false;
      } catch (error) {
        console.warn("获取自启动状态失败:", error);
        // 归属无法确认时保持禁用，不能把正式版启动项当作开发版垃圾清理。
      }
    },
  };
}
