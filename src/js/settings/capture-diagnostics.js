/**
 * 截图几何诊断卡片。
 *
 * 为什么设置页里要有这么一个按钮：几何算错时用户看到的是"覆盖层比屏幕大一圈""画面溢到
 * 隔壁屏"，而判断根因需要的数字全在后端进程里（每块屏上报了什么、舞台图多大、切出来的
 * 裁剪是什么、覆盖层实际被摆成多大）。没有它，一次报障要来回问五轮还问不准。
 *
 * **三条硬约束，改这个文件时不要破：**
 *
 * 1. **只有用户点击才采集。** 打开设置页不会跑诊断——它有一次真实的舞台图请求（约半秒）。
 * 2. **绝不自动上传。** 报告原样显示，复制与提交 issue 都是用户自己按的。
 * 3. **报告用 `textContent` 写进 DOM。** 它来自后端且含用户环境信息，绝不走 innerHTML。
 */

/** 报障入口。带上 issue 模板，用户落地就是那张表，不用自己想该写什么。 */
export const CAPTURE_ISSUE_URL =
  "https://github.com/51hhh/Clippy/issues/new?template=capture-geometry.yml";

/**
 * 报告落盘位置那一行。写不进去时返回 null——报告已经在眼前了，
 * 没落盘不是失败，但也不能假装有个文件在那儿。
 */
export function describeReportPath(report, translate) {
  if (!report?.path) return null;
  return `${translate("settings.captureDiagnostics.savedTo")} ${report.path}`;
}

export function createCaptureDiagnosticsCard({
  noteInput,
  runButton,
  copyButton,
  reportButton,
  pathText,
  output,
  collect,
  copyText,
  openUrl,
  translate,
  notify,
}) {
  let currentReport = null;

  function render(report) {
    currentReport = report;
    // 报告是纯文本，只能这样写进 DOM。
    output.textContent = report.text;
    output.hidden = false;
    const path = describeReportPath(report, translate);
    pathText.textContent = path ?? "";
    pathText.hidden = path === null;
    copyButton.hidden = false;
    reportButton.hidden = false;
  }

  async function run() {
    runButton.disabled = true;
    const original = runButton.textContent;
    runButton.textContent = translate("settings.captureDiagnostics.running");
    try {
      render(await collect(noteInput.value.trim() || null));
    } catch (error) {
      // 诊断本身失败也要说出来：这时用户手里连"哪里出错"都没有。
      output.textContent = String(error);
      output.hidden = false;
      pathText.hidden = true;
      copyButton.hidden = true;
      reportButton.hidden = true;
      currentReport = null;
    } finally {
      runButton.disabled = false;
      runButton.textContent = original;
    }
  }

  async function copy() {
    if (!currentReport) return;
    try {
      await copyText(currentReport.text);
      notify?.(translate("settings.captureDiagnostics.copied"));
    } catch (error) {
      notify?.(String(error));
    }
  }

  /**
   * 先复制再开浏览器。**不把报告塞进 URL**：它有两三千字符，URL 长度限制会把它
   * 悄悄截断，而被截断的诊断报告比没有报告更糟——看起来齐全，数字却少了一半。
   */
  async function reportIssue() {
    if (!currentReport) return;
    await copy();
    try {
      await openUrl(CAPTURE_ISSUE_URL);
    } catch (error) {
      notify?.(String(error));
    }
  }

  runButton.addEventListener("click", () => void run());
  copyButton.addEventListener("click", () => void copy());
  reportButton.addEventListener("click", () => void reportIssue());

  return { run, copy, reportIssue };
}
