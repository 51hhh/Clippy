/**
 * preview/large-text.js — 预览面板对付超大条目的两条闸门
 *
 * 剪贴板里出现几百 KB 甚至几 MB 的一条并不稀奇（复制一整个文件、一份日志、一段
 * base64）。预览面板原来对长度毫无防备，两处会把 webview 那一个线程按住不放：
 *
 * 1. **语言检测**：`hljs.highlightAuto(text)` 会拿注册的 21 种语法**各跑一遍全文**。
 *    判断"这是什么语言"用开头一段就够了，没必要为此把整份文本高亮 21 次。
 * 2. **渲染**：高亮/Markdown/富文本都要产出一份带 span 的 HTML，再过一遍 DOMPurify，
 *    再 `innerHTML` 进 DOM。几 MB 文本产出的 DOM 节点数是六位数，滚也滚不动。
 *
 * 所以检测只看开头 `DETECT_SAMPLE_CHARS`，渲染最多 `MAX_RENDER_CHARS`，超出的部分
 * 由面板追加一行说明。**原文一个字节都没动**——复制、翻译、保存走的都是库里那份，
 * 这两条闸门只影响"面板里画出来多少"。
 */

/**
 * 语言检测取样长度。
 *
 * 32 KiB：足够让任何真实代码的 hljs relevance 远超阈值（阈值是 5，几十行代码就到了），
 * 又只有 21 次全文高亮的一个零头。短条目取样等于全文，行为完全不变。
 */
export const DETECT_SAMPLE_CHARS = 32 * 1024;

/**
 * 渲染上限。
 *
 * 200 KiB 在这块 380 px 宽的面板里已经是几千屏，滚不到底；真要看全文该用编辑器，
 * 不是用剪贴板预览。
 */
export const MAX_RENDER_CHARS = 200 * 1024;

/** 给语言检测用的取样。短文本原样返回，不产生多余拷贝。 */
export function detectionSample(text) {
  if (typeof text !== "string") return "";
  return text.length <= DETECT_SAMPLE_CHARS ? text : text.slice(0, DETECT_SAMPLE_CHARS);
}

/**
 * 给渲染用的正文。
 *
 * @returns {{ body: string, truncated: boolean, omitted: number }}
 *          `truncated` 为真时面板要追加一行说明，否则用户会以为内容就这么多。
 */
export function limitForRender(text) {
  if (typeof text !== "string") return { body: "", truncated: false, omitted: 0 };
  if (text.length <= MAX_RENDER_CHARS) {
    return { body: text, truncated: false, omitted: 0 };
  }
  return {
    body: text.slice(0, MAX_RENDER_CHARS),
    truncated: true,
    omitted: text.length - MAX_RENDER_CHARS,
  };
}
