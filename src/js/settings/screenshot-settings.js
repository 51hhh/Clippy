/**
 * 截图设置：保存目录 + 文件名模板。
 * 两个输入都允许留空，留空表示沿用后端的内置默认值。
 *
 * 早先这里还有"框选之后做什么"的下拉框（直接进编辑器 / 停在工具条）。
 * 现在标注就发生在覆盖层里、工具条恒定显示在选区旁边，这个选项没有语义了。
 */
export function createScreenshotSettings({
  directoryInput,
  browseButton,
  templateInput,
  pickDirectory,
  translate,
  showToast,
}) {
  async function browse() {
    browseButton.disabled = true;
    try {
      const picked = await pickDirectory();
      // 用户取消时后端返回 null，此时保留输入框里已有的目录。
      if (picked) directoryInput.value = picked;
    } catch (error) {
      console.warn("选择截图目录失败:", error);
      showToast(translate("settings.screenshot.browseFailed"));
    } finally {
      browseButton.disabled = false;
    }
  }

  browseButton.addEventListener("click", () => {
    void browse();
  });

  return {
    fill(config) {
      directoryInput.value = config.screenshot_save_dir || "";
      templateInput.value = config.screenshot_filename_template || "";
    },
    getConfig() {
      return {
        screenshot_save_dir: directoryInput.value.trim(),
        screenshot_filename_template: templateInput.value.trim(),
      };
    },
  };
}
