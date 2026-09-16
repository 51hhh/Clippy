/**
 * 快捷键注册失败提示。
 *
 * 后端在启动和保存时都可能注册失败（Wayland Portal 不可用/被拒绝，或 X11 上键位
 * 已被别的程序 grab）。启动阶段的失败早于本页监听，
 * 所以这里既要查 `get_shortcut_failures` 的存量记录，也要接住后续事件。
 */

/** 动作名 → 界面上的称呼 */
const ACTION_KEYS = {
  global: "settings.shortcut.action.global",
  pin: "settings.shortcut.action.pin",
  capture: "settings.shortcut.action.capture",
};

const SESSION_KEYS = {
  wayland: "settings.shortcut.registerFailed.wayland",
  x11: "settings.shortcut.registerFailed.x11",
  native: "settings.shortcut.registerFailed.native",
};

export function createShortcutFailureNotice({ warning, translate }) {
  /** action → failure，同一动作只保留最新一条 */
  const failures = new Map();

  function describe(failure) {
    const action = translate(ACTION_KEYS[failure.action] ?? ACTION_KEYS.global);
    const sessionKey = SESSION_KEYS[failure.session] ?? SESSION_KEYS.x11;
    // reason 是中文日志文案，不进界面；需要细节看日志。
    return translate(sessionKey, { action, shortcut: failure.shortcut || "—" });
  }

  function render() {
    if (!failures.size) {
      warning.textContent = "";
      warning.classList.add("hidden");
      return;
    }
    // 三条动作各一行；换行由 CSS 的 white-space 处理，文本节点写入避免注入。
    warning.textContent = [...failures.values()].map(describe).join("\n");
    warning.classList.remove("hidden");
  }

  return {
    /** 用后端的全量记录替换当前提示 */
    replaceAll(list) {
      failures.clear();
      for (const failure of list ?? []) {
        if (failure?.action) failures.set(failure.action, failure);
      }
      render();
    },
    /** 事件到达：追加或覆盖该动作的提示 */
    add(failure) {
      if (!failure?.action) return;
      failures.set(failure.action, failure);
      render();
    },
    /** 语言切换后按当前语言重绘（文案含插值，data-i18n 无法覆盖） */
    refreshLabels: render,
    /** 保存成功后先清空，随后的失败事件会重新填上 */
    clear() {
      failures.clear();
      render();
    },
  };
}
