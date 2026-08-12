import { keyEventToShortcut } from "../shortcut-recorder.js";

const LISTENER_OPTIONS = { capture: true };

/** 停止录制并等待快捷键恢复后再关闭设置窗口。 */
export async function closeAfterShortcutCleanup(shortcutRecording, closeWindow) {
  await shortcutRecording.stop();
  await closeWindow();
}

/**
 * 统一管理多个互斥快捷键录制器，避免每个录制器复制暂停、恢复和键盘监听逻辑。
 */
export function createShortcutRecordingController({
  recorders,
  pauseShortcuts,
  resumeShortcuts,
  translate,
  eventTarget = window,
  defer = (callback) => setTimeout(callback, 0),
}) {
  let activeKey = null;
  let operationQueue = Promise.resolve();

  function recorderFor(key) {
    const recorder = recorders[key];
    if (!recorder) throw new Error(`Unknown shortcut recorder: ${key}`);
    return recorder;
  }

  async function stopActive() {
    if (!activeKey) return;
    const recorder = recorderFor(activeKey);
    activeKey = null;
    recorder.input.classList.remove("recording");
    recorder.recordButton.textContent = translate("settings.shortcut.record");
    eventTarget.removeEventListener("keydown", onKeyDown, LISTENER_OPTIONS);
    try {
      await resumeShortcuts();
    } catch (error) {
      console.warn(error);
    }
    if (recorder.input.value === translate("settings.shortcut.recording")) {
      recorder.input.value = recorder.getSavedValue() || recorder.defaultValue;
    }
  }

  async function start(key) {
    if (activeKey === key) return;
    if (activeKey) await stopActive();

    const recorder = recorderFor(key);
    activeKey = key;
    try {
      await pauseShortcuts();
    } catch (error) {
      console.warn(error);
    }

    // stop 与切换录制器都通过同一队列执行，因此 pause 完成后才可能恢复。
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

    const shortcut = keyEventToShortcut(event);
    if (!shortcut || !activeKey) return;

    const recordedKey = activeKey;
    const recorder = recorderFor(recordedKey);
    recorder.input.value = shortcut;
    defer(() => {
      if (activeKey === recordedKey) void stop();
    });

    if (!recorder.checkConflict) return;
    try {
      const conflict = await recorder.checkConflict(shortcut);
      recorder.warning?.classList.toggle("hidden", !conflict);
    } catch (error) {
      console.warn(error);
    }
  }

  for (const [key, recorder] of Object.entries(recorders)) {
    recorder.recordButton.addEventListener("click", () => {
      const operation = enqueue(() => (activeKey === key ? stopActive() : start(key)));
      operation.catch(console.warn);
    });
    recorder.clearButton.addEventListener("click", () => {
      recorder.input.value = recorder.getSavedValue() || recorder.defaultValue;
      recorder.warning?.classList.add("hidden");
      if (activeKey === key) void stop();
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
      }
    },
    stop,
    get activeKey() {
      return activeKey;
    },
  };
}
