/**
 * preview/classify.js — 剪贴板内容类型的唯一判定处
 *
 * 之前类型有两套标准：列表行显示后端 `content_type`（只有 text/html/image），
 * 预览面板另跑一整串内容嗅探。同一条 HTML 片段因此一边被叫 HTML、一边被叫 YAML。
 * 现在列表行不显示类型，类型只由这张表判定一次，文案由命中的渲染器写进
 * `#preview-type-badge`（badge 文案和渲染方式天然是一回事，分开写必然再次分叉）。
 *
 * 表是有序的，先匹配先赢，顺序本身就是语义：JWT 必须早于可逆编码，
 * 否则三段 Base64 会被当成普通 Base64；URL 卡片排在最前是因为它不需要
 * hljs/marked/DOMPurify，能省掉一次动态 import。
 *
 * 每条规则：
 *   - `guard`     廉价预筛（长度/前缀），只为跳过昂贵正则，不承担语义
 *   - `detect`    真正的检测器，返回假值即不匹配；返回值透传给 `args`
 *   - `renderer`  渲染器名，或由 detect 结果决定渲染器的函数
 *   - `args`      渲染器实参，默认 `[trimmed]`
 *   - `needsLibs` 渲染前必须等 hljs/marked/DOMPurify 加载完
 *
 * 表里判不出来的（Markdown、代码高亮、HTML 富文本、纯文本）留给 preview-panel.js
 * 的异步尾段——它们要么依赖延迟加载的库，要么依赖再拉一次 IPC 详情。
 */

import {
  isUrl, isJson, isJwt, detectEncoding, identifyHash,
  detectEncrypted, isColor, isTimestamp, isUuid, isIpAddress,
  isEmail, isMacAddress, isCron, isDateString, isSemver,
  isNumberBase, isGradient, isDataSize, isRegex, isCoordinate,
  isMimeType, isMathExpr, isHttpStatus,
} from "./detectors.js";

export const CLASSIFY_RULES = [
  { kind: "url", renderer: "renderUrlCard", guard: (s) => s.length > 0, detect: isUrl },
  { kind: "json", renderer: "renderJson", needsLibs: true, guard: (s) => s.length > 1, detect: isJson },
  // JWT 早于编码检测：三段 Base64 否则会被 detectEncoding 抢走
  { kind: "jwt", renderer: "renderJwt", needsLibs: true, guard: (s) => s.length > 30, detect: isJwt },
  // 可逆编码（Base64 / URL 编码 / HTML 实体 / Unicode / Hex），base64 图片另走一个渲染器
  {
    kind: "encoding",
    renderer: (result) => (result.type === "base64-image" ? "renderBase64Image" : "renderEncoded"),
    detect: detectEncoding,
    args: (_trimmed, result) => [result],
  },
  // 哈希不可逆，只标注类型
  {
    kind: "hash",
    renderer: "renderHash",
    guard: (s) => s.length >= 32 && s.length <= 200,
    detect: identifyHash,
    args: (trimmed, hashType) => [trimmed, hashType],
  },
  // 加密内容允许在预览里输入密钥解密
  {
    kind: "encrypted",
    renderer: "renderEncrypted",
    guard: (s) => s.length >= 24,
    detect: detectEncrypted,
    args: (trimmed, encryptType) => [trimmed, encryptType],
  },
  { kind: "color", renderer: "renderColor", guard: (s) => s.length <= 50, detect: isColor },
  { kind: "timestamp", renderer: "renderTimestamp", detect: isTimestamp },
  { kind: "uuid", renderer: "renderUuid", guard: (s) => s.length >= 32 && s.length <= 39, detect: isUuid },
  { kind: "ip", renderer: "renderIpAddress", guard: (s) => s.length >= 7 && s.length <= 45, detect: isIpAddress },
  { kind: "email", renderer: "renderEmail", guard: (s) => s.length >= 5 && s.length <= 254, detect: isEmail },
  { kind: "mac", renderer: "renderMac", guard: (s) => s.length >= 17 && s.length <= 23, detect: isMacAddress },
  { kind: "cron", renderer: "renderCron", guard: (s) => s.length >= 9, detect: isCron },
  { kind: "date", renderer: "renderDate", guard: (s) => s.length >= 8 && s.length <= 40, detect: isDateString },
  { kind: "semver", renderer: "renderSemver", guard: (s) => s.length >= 5 && s.length <= 60, detect: isSemver },
  { kind: "number-base", renderer: "renderNumberBase", guard: (s) => s.length >= 3 && s.length <= 66, detect: isNumberBase },
  { kind: "gradient", renderer: "renderGradient", guard: (s) => s.length >= 20, detect: isGradient },
  { kind: "data-size", renderer: "renderDataSize", guard: (s) => s.length >= 2 && s.length <= 20, detect: isDataSize },
  { kind: "regex", renderer: "renderRegex", guard: (s) => s.length >= 3 && s.startsWith("/"), detect: isRegex },
  { kind: "coordinate", renderer: "renderCoordinate", guard: (s) => s.length >= 5 && s.length <= 40, detect: isCoordinate },
  {
    kind: "mime-type",
    renderer: "renderMimeType",
    guard: (s) => s.length >= 3 && s.length <= 100 && s.includes("/"),
    detect: isMimeType,
  },
  { kind: "math", renderer: "renderMathExpr", guard: (s) => s.length >= 3 && s.length <= 100, detect: isMathExpr },
  { kind: "http-status", renderer: "renderHttpStatus", guard: (s) => s.length === 3, detect: isHttpStatus },
];

/**
 * 按表顺序判定内容类型。
 *
 * @param {string} trimmed 已 trim 的剪贴板文本
 * @returns {null | { kind: string, renderer: string, needsLibs: boolean, args: unknown[] }}
 *          `null` 表示这张表判不出来，交给 preview-panel.js 的异步尾段
 */
export function classifyText(trimmed) {
  for (const rule of CLASSIFY_RULES) {
    if (rule.guard && !rule.guard(trimmed)) continue;
    const detected = rule.detect(trimmed);
    if (!detected) continue;
    return {
      kind: rule.kind,
      renderer: typeof rule.renderer === "function" ? rule.renderer(detected) : rule.renderer,
      needsLibs: rule.needsLibs === true,
      args: rule.args ? rule.args(trimmed, detected) : [trimmed],
    };
  }
  return null;
}
