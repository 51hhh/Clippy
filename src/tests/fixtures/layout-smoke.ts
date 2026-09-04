/**
 * layout-smoke.ts — 主窗口预览/翻译区的真实布局校验
 *
 * jsdom 没有布局引擎，量不出"翻译区被挤出窗口后被 overflow: hidden 裁掉"这类问题，
 * 所以这条 smoke 在真实浏览器里跑：直接取 index.html 的结构（结构与类名因此不会和产品分叉），
 * 把翻译挂载点填满内容，再断言几何关系。由 scripts/smoke-layout.sh 读像素判定成败。
 */

import "../../styles/themes.css";
import "../../styles/base.css";
import "../../styles/components.css";
// 结构直接取产品 index.html：用 ?raw 而不是 fetch，才能同步跑完——
// headless Firefox 的 --screenshot 不等待顶层 await，异步版本会拍到空白页。
import indexMarkup from "../../index.html?raw";

function assert(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function element<T extends HTMLElement>(root: ParentNode, selector: string): T {
  const found = root.querySelector<T>(selector);
  if (!found) throw new Error(`missing element: ${selector}`);
  return found;
}

/** 用 index.html 真实的 #app 结构，避免 fixture 自己抄一份很快就过期的 DOM */
function mountProductLayout(): HTMLElement {
  const parsed = new DOMParser().parseFromString(indexMarkup, "text/html");
  const app = element<HTMLElement>(parsed, "#app");
  document.body.replaceChildren(document.adoptNode(app));
  return app;
}

/** 翻译面板的真实结构由 React 渲染，这里只需要一个"内容超长"的等价体 */
function fillTranslationPanel(host: HTMLElement): HTMLElement {
  const panel = document.createElement("section");
  panel.className = "translation-panel";
  for (let index = 0; index < 40; index += 1) {
    const row = document.createElement("p");
    row.textContent = `translation result line ${index} — long enough to need scrolling`;
    panel.append(row);
  }
  host.replaceChildren(panel);
  return panel;
}

function fillPreviewContent(content: HTMLElement): void {
  for (let index = 0; index < 40; index += 1) {
    const row = document.createElement("p");
    row.textContent = `preview line ${index}`;
    content.append(row);
  }
}

function verifyPreviewAndTranslationShareTheColumn(app: HTMLElement): void {
  const preview = element<HTMLElement>(app, "#preview-panel");
  preview.classList.remove("hidden");
  const host = element<HTMLElement>(app, "#translation-react-root");
  const content = element<HTMLElement>(app, "#preview-content");
  fillPreviewContent(content);
  const panel = fillTranslationPanel(host);

  const previewBox = preview.getBoundingClientRect();
  const hostBox = host.getBoundingClientRect();
  const panelBox = panel.getBoundingClientRect();
  const contentBox = content.getBoundingClientRect();

  // 1. 翻译区不能被顶出预览面板（旧问题：挂载点无样式 → 无法收缩 → 下半截被裁掉）
  assert(panelBox.bottom <= previewBox.bottom + 1,
    `translation panel overflows the preview panel: ${panelBox.bottom} > ${previewBox.bottom}`);
  assert(hostBox.bottom <= previewBox.bottom + 1,
    `translation host overflows the preview panel: ${hostBox.bottom} > ${previewBox.bottom}`);

  // 2. 高度上限必须真的生效（百分比 max-height 要有确定高度的父级才算）
  assert(hostBox.height <= previewBox.height * 0.55 + 1,
    `translation host ignored its max-height: ${hostBox.height} of ${previewBox.height}`);

  // 3. 预览内容不能被翻译区压成 0
  assert(contentBox.height >= 96,
    `preview content collapsed: ${contentBox.height}`);
  assert(contentBox.bottom <= hostBox.top + 1,
    `preview content overlaps the translation area: ${contentBox.bottom} > ${hostBox.top}`);

  // 4. 超长译文靠自身滚动而不是溢出
  assert(panel.scrollHeight > panel.clientHeight,
    `translation panel is not scrollable: ${panel.scrollHeight} vs ${panel.clientHeight}`);
  assert(content.scrollHeight > content.clientHeight,
    `preview content is not scrollable: ${content.scrollHeight} vs ${content.clientHeight}`);
}

/** 大图和 OCR 必须是一篇连续文档：只有 preview-content 拥有纵向滚动。 */
function verifyImageAndOcrUseOneScroller(app: HTMLElement): void {
  const content = element<HTMLElement>(app, "#preview-content");
  content.className = "preview-content preview-content--image";
  content.replaceChildren();

  const image = document.createElement("img");
  image.alt = "large clipboard preview";
  image.width = 1200;
  image.height = 900;

  const ocr = document.createElement("div");
  ocr.className = "preview-ocr-result";
  ocr.dataset.status = "done";
  const text = document.createElement("pre");
  text.textContent = Array.from(
    { length: 80 },
    (_, index) => `OCR line ${index + 1}: selectable recognized text`,
  ).join("\n");
  ocr.append(text);
  content.append(image, ocr);

  const contentStyle = getComputedStyle(content);
  const ocrStyle = getComputedStyle(ocr);
  assert(contentStyle.overflowY === "auto",
    `preview content lost its scroll ownership: ${contentStyle.overflowY}`);
  assert(ocrStyle.overflowY === "visible",
    `OCR created a nested scroller: ${ocrStyle.overflowY}`);
  assert(ocrStyle.maxHeight === "none",
    `OCR is still height-capped: ${ocrStyle.maxHeight}`);
  assert(ocr.scrollHeight <= ocr.clientHeight + 1,
    `OCR content is internally clipped: ${ocr.scrollHeight} > ${ocr.clientHeight}`);
  assert(content.scrollHeight > content.clientHeight,
    `large image/OCR document does not scroll: ${content.scrollHeight} vs ${content.clientHeight}`);

  const imageBox = image.getBoundingClientRect();
  const ocrBox = ocr.getBoundingClientRect();
  assert(imageBox.bottom <= ocrBox.top + 1,
    `image overlaps OCR content: ${imageBox.bottom} > ${ocrBox.top}`);

  content.scrollTop = content.scrollHeight;
  const contentBox = content.getBoundingClientRect();
  const scrolledOcrBox = ocr.getBoundingClientRect();
  assert(scrolledOcrBox.bottom <= contentBox.bottom + 1,
    `OCR tail is unreachable through preview scroll: ${scrolledOcrBox.bottom} > ${contentBox.bottom}`);
}

function verifyCodecPanelKeepsListWidth(app: HTMLElement): void {
  const codec = element<HTMLElement>(app, "#codec-panel");
  codec.classList.remove("hidden");
  const list = element<HTMLElement>(app, "#list-panel");
  // 三栏都在场时列表不能被挤窄（窗口宽度由 window_controller 按面板数放大）
  assert(Math.round(list.getBoundingClientRect().width) === 380,
    `list panel was squeezed: ${list.getBoundingClientRect().width}`);
  assert(list.tabIndex === -1, "list panel must stay focusable for the keyboard state machine");
}

/** 虚拟列表的偏移算法依赖这两个固定高度，必须用真实布局引擎校验 CSS 契约。 */
function verifyVirtualRowHeights(app: HTMLElement): void {
  const host = element<HTMLElement>(app, "#clipboard-react-root");
  const list = document.createElement("main");
  list.className = "clip-list";
  const content = document.createElement("div");
  content.className = "clip-list-virtual-content";
  const textRow = document.createElement("div");
  textRow.className = "clip-row";
  const imageRow = document.createElement("div");
  imageRow.className = "clip-row clip-row--image";
  const fillRow = (row: HTMLElement, image: boolean) => {
    const main = document.createElement("div");
    main.className = "clip-row-main";
    const preview = document.createElement("div");
    preview.className = `clip-row-preview${image ? " clip-row-preview--image" : ""}`;
    if (image) {
      const thumbnail = document.createElement("span");
      thumbnail.className = "clip-row-thumb";
      preview.append(thumbnail);
    } else {
      preview.textContent = "two-line clipboard preview long enough to wrap without overflowing its fixed row";
    }
    const meta = document.createElement("div");
    meta.className = "clip-row-meta";
    meta.textContent = "24 B · now";
    main.append(preview, meta);
    row.append(main);
  };
  fillRow(textRow, false);
  fillRow(imageRow, true);
  content.append(textRow, imageRow);
  list.append(content);
  host.replaceChildren(list);

  assert(textRow.getBoundingClientRect().height === 77,
    `text row height drifted: ${textRow.getBoundingClientRect().height}`);
  assert(imageRow.getBoundingClientRect().height === 87,
    `image row height drifted: ${imageRow.getBoundingClientRect().height}`);
  assert(textRow.scrollHeight <= textRow.clientHeight,
    `text row content overflowed: ${textRow.scrollHeight} > ${textRow.clientHeight}`);
  assert(imageRow.scrollHeight <= imageRow.clientHeight,
    `image row content overflowed: ${imageRow.scrollHeight} > ${imageRow.clientHeight}`);
}

/**
 * 产品 CSS 会覆盖 body 背景，所以结果画在一个独立浮层上供脚本读像素。
 * 失败时把原因画进浮层：headless 截图模式读不到 console，截图本身就是唯一的诊断信息。
 */
function paint(color: string, reason?: string): void {
  const verdict = document.createElement("div");
  verdict.style.cssText = `position: fixed; inset: 0; z-index: 99999; background: ${color};`
    + "color: #000; font: 13px/1.4 monospace; padding: 8px; white-space: pre-wrap;";
  if (reason) verdict.textContent = reason;
  document.body.append(verdict);
}

try {
  const app = mountProductLayout();
  verifyPreviewAndTranslationShareTheColumn(app);
  verifyImageAndOcrUseOneScroller(app);
  verifyCodecPanelKeepsListWidth(app);
  verifyVirtualRowHeights(app);
  document.documentElement.dataset.layoutSmoke = "passed";
  paint("#00d000");
} catch (error) {
  document.documentElement.dataset.layoutSmoke = "failed";
  document.body.title = String(error);
  console.error(String(error));
  paint("#d00000", String(error));
}
