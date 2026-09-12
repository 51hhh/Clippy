import { keyEventToShortcut, normalizeShortcut } from "../shortcut-recorder.js";

const LISTENER_OPTIONS = { capture: true };

/** 冲突提示文案：Clippy 自己的另一个动作与桌面已有绑定要分开说 */
const CONFLICT_KEYS = {
  clippy: "settings.shortcut.conflictSelf",
  desktop: "settings.shortcut.conflict",
};

/** 停止录制并等待快捷键恢复后再关闭设置窗口。 */
export async function closeAfterShortcutCleanup(shortcutRecording, closeWindow) {
  await shortcutRecording.stop();
  await closeWindow();
}

/** 显式 Save 必须先恢复；Portal 接纳请求仍按 pending 呈现。 */
export async function saveAfterShortcutCleanup(shortcutRecording, save) {
  const restored = await shortcutRecording.stop();
  const outcome = await save();
  return restored?.shortcut_status === "pending" && outcome?.shortcut_status === "unchanged"
    ? { ...outcome, shortcut_status: "pending" } : outcome;
}

/**
 * 统一管理多个互斥快捷键录制器，避免每个录制器复制暂停、恢复和键盘监听逻辑。
 */
export function createShortcutRecordingController({
  recorders,
  pauseShortcuts,
  resumeShortcuts,
  translate,
  notify = console.warn,
  metaModifier = () => "Super",
  eventTarget = window,
  defer = (callback) => setTimeout(callback, 0),
}) {
  let activeKey = null;
  let restoreKey = null;
  let restoreOutcome = null;
  let recordingBeforeValue = "";
  let recordedValue = false;
  let recordingGeneration = 0;
  let operationQueue = Promise.resolve();

  function recorderFor(key) {
    const recorder = recorders[key];
    if (!recorder) throw new Error(`Unknown shortcut recorder: ${key}`);
    return recorder;
  }

  async function stopActive() {
    if (activeKey) {
      const recorder = recorderFor(activeKey);
      activeKey = null;
      recorder.input.classList.remove("recording");
      eventTarget.removeEventListener("keydown", onKeyDown, LISTENER_OPTIONS);
      if (!recordedValue) recorder.input.value = recordingBeforeValue;
    }
    if (!restoreKey) return restoreOutcome;
    const recorder = recorderFor(restoreKey);
    try {
      restoreOutcome = await resumeShortcuts();
    } catch (error) {
      showRestoreFailure(recorder);
      throw error;
    }
    restoreKey = null;
    recorder.recordButton.textContent = translate("settings.shortcut.record");
    if (recorder.warning?.dataset.i18n === "settings.shortcut.restoreFailed") hideWarning(recorder);
    return restoreOutcome;
  }

  async function start(key) {
    if (activeKey === key) return;
    if (activeKey || restoreKey) await stopActive();

    const recorder = recorderFor(key);
    recordingGeneration += 1;
    restoreKey = key;
    restoreOutcome = null;
    try {
      await pauseShortcuts();
    } catch (error) {
      // 暂停也可能只完成了一部分，必须先恢复才能录制或保存。
      showRestoreFailure(recorder);
      throw error;
    }

    // stop 与切换录制器都通过同一队列执行，因此 pause 完成后才可能恢复。
    activeKey = key;
    recordingBeforeValue = recorder.input.value;
    recordedValue = false;
    recorder.input.value = translate("settings.shortcut.recording");
    recorder.input.classList.add("recording");
    recorder.recordButton.textContent = translate("settings.shortcut.stop");
    recorder.warning?.classList.add("hidden");
    eventTarget.addEventListener("keydown", onKeyDown, LISTENER_OPTIONS);
  }

  function enqueue(operation) {
    const result = operationQueue.then(operation, operation);
    operationQueue = result.catch(() => {});
    return result;
  }

  function stop() {
    return enqueue(stopActive);
  }

  async function onKeyDown(event) {
    event.preventDefault();
    event.stopImmediatePropagation();

    const shortcut = keyEventToShortcut(event, metaModifier());
    if (!shortcut || !activeKey) return;

    const recordedKey = activeKey;
    const generation = recordingGeneration;
    const recorder = recorderFor(recordedKey);
    recordedValue = true;
    recorder.input.value = shortcut;
    defer(() => {
      if (activeKey === recordedKey) void stop().catch(notify);
    });

    // Clippy 三个动作之间的冲突在前端判断：输入框里可能是还没保存的值，
    // 后端看到的配置是旧的。
    const own = ownConflict(recordedKey, shortcut);
    if (own) {
      showWarning(recorder, "clippy");
      return;
    }
    if (!recorder.checkConflict) {
      hideWarning(recorder);
      return;
    }
    try {
      const result = await recorder.checkConflict(shortcut);
      if (generation !== recordingGeneration || recorder.warning?.dataset.i18n === "settings.shortcut.restoreFailed") return;
      const source = conflictSource(result);
      source ? showWarning(recorder, source) : hideWarning(recorder);
    } catch (error) {
      console.warn(error);
    }
  }

  /** 另一个录制器当前的值是否就是这个组合 */
  function ownConflict(recordedKey, shortcut) {
    const target = normalizeShortcut(shortcut);
    if (!target) return false;
    return Object.entries(recorders).some(([key, other]) =>
      key !== recordedKey && normalizeShortcut(other.input.value) === target);
  }

  /** 后端返回 `{conflicted, source}`；老式布尔值按桌面冲突处理 */
  function conflictSource(result) {
    if (result && typeof result === "object") {
      return result.conflicted ? (CONFLICT_KEYS[result.source] ? result.source : "desktop") : null;
    }
    return result ? "desktop" : null;
  }

  function showWarning(recorder, source) {
    const warning = recorder.warning;
    if (!warning) return;
    const key = CONFLICT_KEYS[source] ?? CONFLICT_KEYS.desktop;
    // data-i18n 一起改，语言切换时刷新到的仍是当前这条提示
    warning.dataset.i18n = key;
    warning.textContent = translate(key);
    warning.classList.remove("hidden");
  }

  function hideWarning(recorder) {
    recorder.warning?.classList.add("hidden");
  }

  function showRestoreFailure(recorder) {
    recorder.recordButton.textContent = translate("settings.shortcut.retry");
    if (recorder.warning) {
      recorder.warning.dataset.i18n = "settings.shortcut.restoreFailed";
      recorder.warning.textContent = translate("settings.shortcut.restoreFailed");
      recorder.warning.classList.remove("hidden");
    }
  }

  for (const [key, recorder] of Object.entries(recorders)) {
    recorder.recordButton.addEventListener("click", () => {
      const operation = enqueue(() => (activeKey === key || restoreKey === key ? stopActive() : start(key)));
      operation.catch(notify);
    });
    recorder.clearButton.addEventListener("click", () => {
      if (activeKey === key) recordedValue = true;
      recorder.input.value = recorder.getSavedValue() || recorder.defaultValue;
      recorder.warning?.classList.add("hidden");
      if (activeKey === key || restoreKey === key) void stop().catch(notify);
    });
  }

  return {
    setValues(values) {
      for (const [key, value] of Object.entries(values)) {
        recorderFor(key).input.value = value;
      }
    },
    getValues() {
      return Object.fromEntries(
        Object.entries(recorders).map(([key, recorder]) => [key, recorder.input.value.trim()]),
      );
    },
    refreshLabels() {
      if (activeKey) {
        recorderFor(activeKey).recordButton.textContent = translate("settings.shortcut.stop");
        if (!recordedValue) recorderFor(activeKey).input.value = translate("settings.shortcut.recording");
      } else if (restoreKey) {
        showRestoreFailure(recorderFor(restoreKey));
      }
    },
    stop,
    get activeKey() {
      return activeKey;
    },
    get restorePending() { return restoreKey !== null; },
  };
}
