/**
 * 全局 Pin 快捷键要 pin 哪一条。
 *
 * 规则：**只有面板此刻真的握着键盘焦点时，焦点行才代表用户的意思**，否则一律问后端要
 * 最新一条。两个理由缺一不可：
 *
 *   1. 面板没焦点时那个"焦点行"是上一轮会话留下的残影，不是用户正看着的行。侧栏
 *      （预览/编解码）开着时失焦不会隐藏窗口，前端也就不会 `releaseMemory()`
 *      （见 `window_events.rs::should_hide_on_focus_loss` 与 `app.js::onWindowBlur`），
 *      于是整份列表连焦点一起活过整个截图流程。而 `prependClip` 是**按条目**跟焦点的
 *      （用户正看着的那行不该在眼皮下换成别的内容），新截图插到第 0 行、焦点跟着老条目
 *      挪到第 1 行——此时按 Pin 贴出来的就是上一张图。再截一张就掉到第 2 行。
 *   2. 前端列表缓存本身也会滞后：入库是 watcher 干的（截图点对钩只把 PNG 写进系统
 *      剪贴板），缓存要等 `clip-added` 才更新。后端已经把这个窗口从最多 500 ms 缩到
 *      几毫秒（`clipboard_watcher/wake.rs` 的写入唤醒），但那是让竞争窗口变小，
 *      不是让它消失，所以这里读库而不是读缓存。
 *
 * @param {object|null} focusedClip 列表当前的焦点条目，没有就传 null
 * @param {() => Promise<object[]>} fetchLatestClips 向后端取最新一条（`getClips(null, false, 0, 1)`）
 * @param {boolean} panelFocused 主面板此刻是否握着键盘焦点（`document.hasFocus()`）。
 *   默认 `false`：漏传时退化成"问后端"，慢一点但绝不会贴错。
 * @returns {Promise<object|null>}
 */
export async function resolvePinTarget(focusedClip, fetchLatestClips, panelFocused = false) {
  if (panelFocused && focusedClip) return focusedClip;
  const clips = await fetchLatestClips();
  return (Array.isArray(clips) && clips[0]) || null;
}
