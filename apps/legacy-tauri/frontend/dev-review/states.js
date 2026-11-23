// 仅 Vite dev 入口：真实更新控制器 + 合成服务，不访问系统、网络或原生 IPC。
import { createUpdateModal } from "../js/update-modal.js";
import { shortcutSaveErrorMessage } from "../js/settings/shortcut-recording.js";
import * as i18n from "../i18n/i18n.js";
import "../styles/themes.css";
import "../styles/base.css";
import "../styles/components.css";
import "./review.css";
import "./states.css";

if (!import.meta.env.DEV) throw new Error("Development review is unavailable in production");
const params = new URLSearchParams(location.search);
const language = params.get("lang") === "en" ? "en" : "zh-CN";
const zh = language === "zh-CN";
const element = id => document.getElementById(id);
const label = (id, cn, en) => { element(id).textContent = zh ? cn : en; };
i18n.init(language);
document.documentElement.lang = language;
element("language").value = language;
element("language").addEventListener("change", event => {
  params.set("lang", event.target.value);
  location.search = params.toString();
});
element("theme").value = params.get("theme") === "dark" ? "dark" : "light";
document.documentElement.dataset.theme = element("theme").value;
element("theme").addEventListener("change", event => { document.documentElement.dataset.theme = event.target.value; });
label("review-heading", "开发审阅 · 更新状态", "Development review · update states");
label("review-sidebar", "图片侧栏", "Image sidebar");
label("review-pin", "Pin", "Pin");
label("review-capture", "截图", "Capture");
for (const id of ["review-sidebar", "review-pin", "review-capture"]) element(id).href += `?lang=${language}`;
label("review-note", "使用真实更新弹窗与主题。安装、重启和下载均为合成演示，不访问更新服务，不改变本机应用。", "Actual update dialog and themes. Install, restart and download are synthetic demonstrations with no update service access or local application changes.");
label("scenario-label", "场景", "Scenario");
label("platform-label", "安装方式", "Installation");
label("theme-label", "主题", "Theme");
label("reopen", "重新打开", "Reopen");
label("review-background", "关闭弹窗后，点击“重新打开”可恢复当前状态。", "Close the dialog, then select Reopen to restore its current state.");
label("conflict-title", "快捷键冲突反馈", "Shortcut conflict feedback");
element("conflict-message").textContent = shortcutSaveErrorMessage("settings.shortcut.duplicate:global,pin", i18n.t);
const scenarioNames = {
  available: ["可安装", "Available"], installing: ["下载中", "Installing"],
  installed: ["已安装", "Installed"], failed: ["安装失败", "Failed"],
};
for (const option of element("scenario").options) option.textContent = scenarioNames[option.value][zh ? 0 : 1];
for (const option of element("theme").options) option.textContent = option.value === "light" ? (zh ? "浅色" : "Light") : (zh ? "深色" : "Dark");

const listeners = new Set();
let timer = null;
let snapshot = { revision: 0, status: "available", version: "9.9.9", body: zh ? "界面审阅示例\n- 更清晰的图片预览\n- 更稳定的快捷键与更新流程" : "Interface review sample\n- Clearer image previews\n- More reliable shortcut and update flows", install_type: "appimage", downloaded: 0, total: 100 };
function publish(change) {
  snapshot = { ...snapshot, ...change, revision: snapshot.revision + 1 };
  for (const listener of listeners) listener({ ...snapshot });
  element("scenario").value = snapshot.status;
}
const services = {
  checkUpdate: async () => ({ ...snapshot }),
  getAppUpdateState: async () => ({ ...snapshot }),
  onAppUpdateState: async callback => { listeners.add(callback); return () => listeners.delete(callback); },
  downloadAndInstallUpdate: async () => {
    publish({ status: "installing", downloaded: 0 });
    clearInterval(timer);
    timer = setInterval(() => {
      const downloaded = snapshot.downloaded + 10;
      publish({ downloaded, status: downloaded >= 100 ? "installed" : "installing" });
      if (downloaded >= 100) { clearInterval(timer); timer = null; }
    }, 500);
    label("feedback", "模拟安装中：五秒后显示完成。", "Simulated installation: completion appears in five seconds.");
    return { ...snapshot };
  },
  restartApp: async () => { label("feedback", "已模拟重启操作；本机应用没有重启。", "Restart was simulated; the local application was not restarted."); },
  openExternalUrl: async () => { label("feedback", "已模拟打开下载页；未打开外部链接。", "Opening the download page was simulated; no external link was opened."); },
};
const controller = createUpdateModal({ services });
async function showScenario() {
  clearInterval(timer); timer = null;
  const status = element("scenario").value;
  publish({ status, install_type: element("platform").value, downloaded: status === "installing" ? 48 : status === "installed" ? 100 : 0 });
  label("feedback", "可测试：Install、Later、Close、Restart、Tab 和 Escape。安装中仅展示进度；可在上方切换其他场景。", "Try Install, Later, Close, Restart, Tab and Escape. Installing shows progress; switch scenarios using the controls above.");
  await controller.checkForUpdate(true);
}
element("scenario").addEventListener("change", showScenario);
element("platform").addEventListener("change", showScenario);
element("reopen").addEventListener("click", () => { void controller.checkForUpdate(true); });
window.addEventListener("pagehide", () => { clearInterval(timer); controller.dispose(); }, { once: true });
void showScenario();
