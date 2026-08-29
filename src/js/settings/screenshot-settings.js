/** 后端认得的选区提交动作，其余值一律按默认的 editor 处理。 */
const COMMIT_ACTIONS = ["editor", "toolbar"];
const DEFAULT_COMMIT_ACTION = "editor";

function normalizeCommitAction(value) {
  return COMMIT_ACTIONS.includes(value) ? value : DEFAULT_COMMIT_ACTION;
}

/**
 * 截图设置：保存目录 + 文件名模板 + 框选完成后的默认动作。
 * 两个输入都允许留空，留空表示沿用后端的内置默认值。
 */
export function createScreenshotSettings({
  directoryInput,
  browseButton,
  templateInput,
  commitActionControl,
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
      commitActionControl.value = normalizeCommitAction(config.capture_commit_action);
    },
    getConfig() {
      return {
        screenshot_save_dir: directoryInput.value.trim(),
        screenshot_filename_template: templateInput.value.trim(),
        capture_commit_action: normalizeCommitAction(commitActionControl.value),
      };
    },
  };
}
