#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const pass = (id, title) => Object.freeze({ id, title, acceptedStatuses: ["pass"] });
const degraded = (id, title, acceptedReasonCodes) =>
  Object.freeze({
    id,
    title,
    acceptedStatuses: ["expected_degraded"],
    acceptedReasonCodes: Array.isArray(acceptedReasonCodes)
      ? Object.freeze([...acceptedReasonCodes])
      : Object.freeze([acceptedReasonCodes]),
  });

const PORTAL_AUTHORIZATION_OUTCOMES = Object.freeze([
  "portal_select_devices_rejected",
  "portal_start_rejected",
  "portal_keyboard_not_granted",
  "portal_attempt_exhausted",
]);

const COMMON_CASES = Object.freeze([
  pass("clipboard_text", "文本复制、历史记录与粘贴"),
  pass("clipboard_html", "HTML 与纯文本 fallback"),
  pass("clipboard_image", "图片复制、缩略图与快速粘贴"),
  pass("clipboard_monitoring", "监听暂停/恢复、自复制唤醒、SHA-256 去重与历史上限"),
  pass("history_search_favorites", "FTS 搜索、分页、收藏、删除、清理与统计"),
  pass("keyboard_navigation", "列表、搜索、预览、翻译和 codec 面板的完整键盘状态机"),
  pass("codec_tools", "全部编解码、哈希、格式化、解析、时间戳与进制转换"),
  pass("rich_preview_safety", "代码/Markdown/HTML/图片/JWT/编码/哈希/数学/加密/大文本预览与安全边界"),
  pass("global_shortcut", "全局快捷键注册、暂停、恢复与冲突提示"),
  pass("screenshot_area", "区域截图、取消、Copy、Save、Pin 与 Translate"),
  pass("pin_basic", "文本/图片 Pin、移动、缩放、透明度、锁定、工具条、关闭和再次编辑"),
  pass(
    "canvas_tools",
    "16 种画布工具（裁剪/对象/橡皮/文本、八种绘制、四种像素效果）、调整、撤销和重做",
  ),
  pass("editable_png_roundtrip", "可编辑 PNG 保存、重开、继续编辑与扁平导出"),
  pass("system_save_directory", "系统图片目录、自定义目录和不可用 fallback"),
  pass("ocr_platform_behavior", "OCR 探测、平台安装提示和不可用降级"),
  pass("translation_roundtrip", "多服务翻译、语言换向、重试、缓存与历史删除"),
  pass("translation_privacy_tts", "敏感内容阻断、图片仅上传 OCR 文本与 TTS 播放"),
  pass("keyring_no_plaintext", "凭据写入系统 keyring 且失败时不落明文"),
  pass("settings_theme_i18n", "六套主题、自动/中英语言和设置重启持久化"),
  pass("tray_single_instance", "托盘菜单、暂停状态、窗口复用与单实例回调"),
  pass("autostart_roundtrip", "登录启动启用、重启验证与关闭清理"),
  pass("tmux_platform_behavior", "Linux tmux copy-mode 捕获或非 Linux 隐藏与禁用"),
  pass("diagnostics_privacy", "typed 平台能力和诊断不泄露像素、标题、token 或密钥"),
  pass("updater_roundtrip", "安装类型识别、检查更新、下载安装与重启"),
]);

const PROFILES = {
  "linux-gnome-x11": {
    label: "Ubuntu 22.04 GNOME 42 X11",
    operatingSystem: "linux",
    session: "x11",
    cases: [
      pass("auto_paste_regular", "普通应用自动粘贴并恢复焦点"),
      pass("screenshot_window", "窗口命中、边框裁剪与窗口截图"),
      pass("mixed_dpi", "多显示器、负坐标与混合缩放"),
      pass("pin_topmost", "Pin 窗口持续置顶"),
      pass("capture_diagnostics", "截图诊断 I1–I5 与 fixture 输出"),
    ],
  },
  "linux-gnome-wayland": {
    label: "Ubuntu 22.04 GNOME 42 Wayland",
    operatingSystem: "linux",
    session: "wayland",
    cases: [
      pass("auto_paste_authorized", "授权后的 RemoteDesktop 自动粘贴"),
      degraded("auto_paste_denied", "拒绝粘贴授权后保持 copy-only", PORTAL_AUTHORIZATION_OUTCOMES),
      pass("gnome_extension_lifecycle", "窗口扩展未装、待注销、就绪和旧版恢复"),
      pass("screenshot_window", "Shell helper 窗口命中与区域截图 fallback"),
      degraded("absolute_position_limit", "绝对定位限制与 UI 如实降级", "wayland_protocol_limited"),
      degraded("pin_topmost_limit", "永久置顶限制与 UI 如实降级", "wayland_protocol_limited"),
      pass("mixed_dpi", "多显示器、负坐标与混合缩放"),
      pass("capture_diagnostics", "截图诊断 I1–I5 与 fixture 输出"),
    ],
  },
  "linux-gnome-wayland-ubuntu24": {
    label: "Ubuntu 24.04 GNOME Wayland",
    operatingSystem: "linux",
    session: "wayland",
    cases: [
      pass("auto_paste_authorized", "授权后的 RemoteDesktop 自动粘贴"),
      degraded("auto_paste_denied", "拒绝粘贴授权后保持 copy-only", PORTAL_AUTHORIZATION_OUTCOMES),
      pass("gnome_extension_lifecycle", "窗口扩展未装、待注销、就绪和旧版恢复"),
      pass("screenshot_window", "Shell helper 窗口命中与区域截图 fallback"),
      pass("wayland_capture_fallbacks", "Mutter、Shell helper、Portal 与后续截图 fallback"),
      pass("portal_capability_versions", "记录 Portal 各接口版本与实际选择的后端"),
      degraded("absolute_position_limit", "绝对定位限制与 UI 如实降级", "wayland_protocol_limited"),
      degraded("pin_topmost_limit", "永久置顶限制与 UI 如实降级", "wayland_protocol_limited"),
      pass("mixed_dpi", "多显示器、负坐标与混合缩放"),
      pass("capture_diagnostics", "截图诊断 I1–I5 与 fixture 输出"),
    ],
  },
  "linux-kde-wayland": {
    label: "KDE Wayland",
    operatingSystem: "linux",
    session: "wayland",
    cases: [
      pass("portal_shortcut_authorized", "GlobalShortcuts Portal 首次授权、修改和恢复"),
      degraded("portal_shortcut_denied", "快捷键拒绝后逐动作报告", "wayland_portal_permission"),
      pass("auto_paste_authorized", "授权后的 RemoteDesktop 自动粘贴"),
      degraded("auto_paste_denied", "拒绝粘贴授权后保持 copy-only", PORTAL_AUTHORIZATION_OUTCOMES),
      pass("portal_screenshot", "Portal 区域截图与取消恢复"),
      degraded("window_pick_limit", "缺少全局窗口几何时不声称窗口命中", "wayland_protocol_limited"),
      degraded("absolute_position_limit", "绝对定位限制与 UI 如实降级", "wayland_protocol_limited"),
      degraded("pin_topmost_limit", "永久置顶限制与 UI 如实降级", "wayland_protocol_limited"),
    ],
  },
  "linux-wlroots-wayland": {
    label: "wlroots Wayland compositor",
    operatingSystem: "linux",
    session: "wayland",
    cases: [
      pass("area_capture_fallback", "逐输出或 Portal 区域截图 fallback"),
      pass("data_control_clipboard", "data-control 可用时的文本与图片剪贴板"),
      degraded("window_pick_limit", "缺少全局窗口几何时不声称窗口命中", "wayland_protocol_limited"),
      degraded("absolute_position_limit", "绝对定位限制与 UI 如实降级", "wayland_protocol_limited"),
      degraded("pin_topmost_limit", "永久置顶限制与 UI 如实降级", "wayland_protocol_limited"),
      degraded("portal_unavailable", "缺失 Portal 接口时给出稳定原因", "wayland_portal_unavailable"),
    ],
  },
  "windows-10-x64": {
    label: "Windows 10 22H2 x64",
    operatingSystem: "windows",
    session: "native",
    architecture: "x86_64",
    cases: [
      pass("auto_paste_regular", "普通完整性应用自动粘贴并恢复焦点"),
      degraded("auto_paste_high_integrity", "管理员目标保持 copy-only", "windows_integrity_boundary"),
      pass("screenshot_window", "HWND 窗口命中与窗口截图"),
      pass("mixed_dpi", "混合 DPI、多显示器和负坐标"),
      pass("pin_topmost", "原生 topmost 与焦点行为"),
      pass("installer_nsis_msi", "NSIS/MSI 安装、卸载和升级"),
      pass("private_file_acl", "私有目录、旧文件修复与原子替换 ACL"),
    ],
  },
  "windows-11-x64": {
    label: "Windows 11 x64",
    operatingSystem: "windows",
    session: "native",
    architecture: "x86_64",
    cases: [
      pass("auto_paste_regular", "普通完整性应用自动粘贴并恢复焦点"),
      degraded("auto_paste_high_integrity", "管理员目标保持 copy-only", "windows_integrity_boundary"),
      pass("screenshot_window", "HWND 窗口命中与窗口截图"),
      pass("mixed_dpi", "混合 DPI、多显示器和负坐标"),
      pass("pin_topmost", "原生 topmost 与焦点行为"),
      pass("installer_nsis_msi", "NSIS/MSI 安装、卸载和升级"),
      pass("private_file_acl", "私有目录、旧文件修复与原子替换 ACL"),
    ],
  },
  "macos-intel": {
    label: "macOS 11+ Intel",
    operatingSystem: "macos",
    session: "native",
    architecture: "x86_64",
    cases: [
      degraded("screen_recording_undecided", "屏幕录制未决定时只请求一次", "macos_screen_recording_permission"),
      degraded("screen_recording_denied", "屏幕录制拒绝后不反复弹框", "macos_screen_recording_permission"),
      pass("screen_recording_authorized", "授权后无需重启恢复截图与窗口命中"),
      degraded("screen_recording_revoked", "撤销后实时降级且可再次恢复", "macos_screen_recording_permission"),
      degraded("accessibility_undecided", "辅助功能未决定时请求授权", "macos_accessibility_permission"),
      degraded(
        "accessibility_denied",
        "辅助功能拒绝后保持 copy-only",
        "macos_accessibility_permission_required",
      ),
      pass("accessibility_authorized", "授权后自动粘贴并恢复应用"),
      degraded(
        "accessibility_revoked",
        "撤销后实时降级且可再次恢复",
        "macos_accessibility_permission_required",
      ),
      pass("spaces_fullscreen", "Spaces、全屏窗口与辅助窗口层级"),
      pass(
        "adhoc_bundle_boundary",
        "Ad-Hoc 签名、目标架构、未公证边界与首次打开恢复",
      ),
    ],
  },
  "macos-apple-silicon": {
    label: "macOS 11+ Apple Silicon",
    operatingSystem: "macos",
    session: "native",
    architecture: "aarch64",
    cases: [
      degraded("screen_recording_undecided", "屏幕录制未决定时只请求一次", "macos_screen_recording_permission"),
      degraded("screen_recording_denied", "屏幕录制拒绝后不反复弹框", "macos_screen_recording_permission"),
      pass("screen_recording_authorized", "授权后无需重启恢复截图与窗口命中"),
      degraded("screen_recording_revoked", "撤销后实时降级且可再次恢复", "macos_screen_recording_permission"),
      degraded("accessibility_undecided", "辅助功能未决定时请求授权", "macos_accessibility_permission"),
      degraded(
        "accessibility_denied",
        "辅助功能拒绝后保持 copy-only",
        "macos_accessibility_permission_required",
      ),
      pass("accessibility_authorized", "授权后自动粘贴并恢复应用"),
      degraded(
        "accessibility_revoked",
        "撤销后实时降级且可再次恢复",
        "macos_accessibility_permission_required",
      ),
      pass("spaces_fullscreen", "Spaces、全屏窗口与辅助窗口层级"),
      pass(
        "adhoc_bundle_boundary",
        "Ad-Hoc 签名、目标架构、未公证边界与首次打开恢复",
      ),
    ],
  },
};

export const QA_PROFILES = Object.freeze(
  Object.fromEntries(
    Object.entries(PROFILES).map(([key, value]) => [
      key,
      Object.freeze({ ...value, cases: Object.freeze([...value.cases]) }),
    ]),
  ),
);

export function casesForProfile(profileId) {
  const profile = QA_PROFILES[profileId];
  if (!profile) throw new Error(`未知 QA profile: ${profileId}`);
  return [...COMMON_CASES, ...profile.cases];
}

export function createQaTemplate({ profileId, commit, appVersion }) {
  const profile = QA_PROFILES[profileId];
  if (!profile) throw new Error(`未知 QA profile: ${profileId}`);
  return {
    schemaVersion: 1,
    profile: profileId,
    commit,
    appVersion,
    testedAt: null,
    environment: {
      operatingSystem: profile.operatingSystem,
      osVersion: "",
      session: profile.session,
      desktopEnvironment: profile.label,
      architecture: profile.architecture ?? "",
    },
    results: casesForProfile(profileId).map((testCase) => ({
      id: testCase.id,
      title: testCase.title,
      acceptedStatuses: testCase.acceptedStatuses,
      acceptedReasonCodes: testCase.acceptedReasonCodes ?? [],
      status: "not_run",
      observedReasonCode: null,
      observation: "",
      evidence: [],
    })),
  };
}

function validateMetadata(record, errors) {
  if (record.schemaVersion !== 1) errors.push("schemaVersion 必须是 1");
  if (!/^[0-9a-f]{40}$/i.test(record.commit ?? "")) errors.push("commit 必须是完整 40 位 SHA");
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(record.appVersion ?? "")) {
    errors.push("appVersion 必须是 SemVer");
  }
  if (
    typeof record.testedAt !== "string" ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(record.testedAt) ||
    Number.isNaN(Date.parse(record.testedAt))
  ) {
    errors.push("testedAt 必须是 RFC 3339 时间");
  }
}

export function verifyQaRecord(record) {
  const errors = [];
  const profile = QA_PROFILES[record?.profile];
  if (!profile) return { passed: false, errors: [`未知 QA profile: ${record?.profile}`], results: [] };
  validateMetadata(record, errors);

  const environment = record.environment ?? {};
  if (environment.operatingSystem !== profile.operatingSystem) errors.push("operatingSystem 与 profile 不符");
  if (environment.session !== profile.session) errors.push("session 与 profile 不符");
  if (!String(environment.osVersion ?? "").trim()) errors.push("osVersion 不能为空");
  if (!String(environment.desktopEnvironment ?? "").trim()) {
    errors.push("desktopEnvironment 不能为空");
  }
  if (!String(environment.architecture ?? "").trim()) errors.push("architecture 不能为空");
  if (profile.architecture && environment.architecture !== profile.architecture) {
    errors.push("architecture 与 profile 不符");
  }

  const actualResults = Array.isArray(record.results) ? record.results : [];
  const counts = new Map();
  for (const result of actualResults) counts.set(result.id, (counts.get(result.id) ?? 0) + 1);
  const expectedCases = casesForProfile(record.profile);
  const expectedIds = new Set(expectedCases.map((testCase) => testCase.id));
  for (const id of counts.keys()) if (!expectedIds.has(id)) errors.push(`存在未知场景: ${id}`);

  const results = expectedCases.map((testCase) => {
    const matches = actualResults.filter((result) => result.id === testCase.id);
    const result = matches[0];
    if (matches.length !== 1) errors.push(`${testCase.id} 必须且只能出现一次`);
    if (!result) return { id: testCase.id, passed: false, status: "missing" };

    let passed = testCase.acceptedStatuses.includes(result.status);
    if (!passed) errors.push(`${testCase.id} 状态 ${result.status} 不满足 ${testCase.acceptedStatuses.join("/")}`);
    if (
      testCase.acceptedReasonCodes?.length &&
      !testCase.acceptedReasonCodes.includes(result.observedReasonCode)
    ) {
      errors.push(
        `${testCase.id} 必须观测 reason code ${testCase.acceptedReasonCodes.join("/")}`,
      );
      passed = false;
    }
    if (!String(result.observation ?? "").trim()) {
      errors.push(`${testCase.id} 缺少 observation`);
      passed = false;
    }
    if (!Array.isArray(result.evidence) || !result.evidence.some((item) => String(item).trim())) {
      errors.push(`${testCase.id} 缺少 evidence`);
      passed = false;
    }
    return { id: testCase.id, passed, status: result.status };
  });
  return { passed: errors.length === 0 && results.every((result) => result.passed), errors, results };
}

export function formatQaReport(record, verification) {
  const profile = QA_PROFILES[record.profile];
  const lines = [
    `# 真机 QA：${profile?.label ?? record.profile}`,
    "",
    `- Commit: \`${record.commit}\``,
    `- App version: \`${record.appVersion}\``,
    `- Tested at: \`${record.testedAt}\``,
    `- Result: **${verification.passed ? "PASS" : "FAIL"}**`,
    "",
    "| Scenario | Status | Reason | Evidence |",
    "|---|---|---|---|",
  ];
  for (const result of record.results ?? []) {
    lines.push(
      `| ${result.id} | ${result.status} | ${result.observedReasonCode ?? "—"} | ${(result.evidence ?? []).join("<br>") || "—"} |`,
    );
  }
  if (verification.errors.length) {
    lines.push("", "## Errors", "", ...verification.errors.map((error) => `- ${error}`));
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

function parseOptions(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    if (!argv[index]?.startsWith("--") || argv[index + 1] === undefined) {
      throw new Error(`参数必须使用 --name value：${argv[index] ?? "<missing>"}`);
    }
    options[argv[index].slice(2)] = argv[index + 1];
  }
  return options;
}

function main() {
  const [command, ...argv] = process.argv.slice(2);
  const options = parseOptions(argv);
  if (command === "template") {
    if (!options.profile || !options.sha || !options.version || !options.output) {
      throw new Error("template 需要 --profile、--sha、--version 和 --output");
    }
    const template = createQaTemplate({
      profileId: options.profile,
      commit: options.sha,
      appVersion: options.version,
    });
    writeFileSync(resolve(options.output), `${JSON.stringify(template, null, 2)}\n`, "utf8");
    return;
  }
  if (command === "verify") {
    if (!options.input) throw new Error("verify 需要 --input");
    const record = JSON.parse(readFileSync(resolve(options.input), "utf8"));
    const verification = verifyQaRecord(record);
    const report = formatQaReport(record, verification);
    process.stdout.write(report);
    if (options.output) writeFileSync(resolve(options.output), report, "utf8");
    if (!verification.passed) process.exitCode = 1;
    return;
  }
  throw new Error("用法：manual-qa.mjs template|verify [options]");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(`真机 QA 工具失败：${error.message}`);
    process.exitCode = 1;
  }
}
