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
// 结构和数据都在本模块内；不 fetch，不等待图片/IPC/网络，完成微任务后即可截图。
import indexMarkup from "../../index.html?raw";
import React from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { createContentRenderers } from "../../js/preview/content-renderers.js";
import { createImageTranslationView, revealImageTranslation } from "../../js/preview/reveal-translation.js";
import { TranslationPanel } from "../../react/main/TranslationPanel";
import { ClipboardRow } from "../../react/main/ClipboardRow";
import { TranslationStore, type TranslationServices } from "../../react/main/translationStore";
import type { AppConfig, ClipItem } from "../../js/ipc-types";

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

/** 使用产品 renderer + React portal；图片/OCR/译文只有 preview-scroll 滚动。 */
async function verifyImageAndOcrUseOneScroller(app: HTMLElement): Promise<void> {
  const preview = element<HTMLElement>(app, "#preview-panel");
  const content = element<HTMLElement>(app, "#preview-content");
  const host = element<HTMLElement>(app, "#translation-react-root");
  const scroll = element<HTMLElement>(app, ".preview-scroll");
  preview.classList.add("preview-panel--image");
  content.className = "preview-content";
  content.replaceChildren(); host.replaceChildren();
  const imageView = createImageTranslationView();
  const canvas = document.createElement("canvas");
  canvas.width = 640; canvas.height = 1800;
  canvas.getContext("2d")!.fillRect(0, 0, 640, 1800);
  const png = canvas.toDataURL("image/png").split(",")[1];
  const recognized = Array.from({ length: 80 }, (_, i) => `OCR line ${i + 1}: selectable recognized text`).join("\n");
  const translated = Array.from({ length: 80 }, (_, i) => `Translation line ${i + 1}: complete result`).join("\n");
  const clip: ClipItem = { id: 1, content_type: "image", text_content: null, html_content: null,
    image_data: null, content_hash: "fixture", is_favorite: false, is_sensitive: false, created_at: 1, byte_size: 2048 };
  const config = { ocr_enabled: true, ocr_result_mode: "preview", translation_target_language: "en",
    translation_services: [{ provider: "libretranslate", enabled: true, endpoint: "https://example.test", model: "", region: "", project: "" }],
  } as AppConfig;
  let requests = 0;
  const services: TranslationServices = {
    copyText: async () => {}, translationHistory: async () => [],
    speakClip: async () => ({ mime_type: "audio/wav", audio_base64: "" }),
    speakText: async () => ({ mime_type: "audio/wav", audio_base64: "" }),
    translateClip: async () => { requests++; return { request_id: 1, services: [{ status: "ok", provider: "libretranslate",
      translated_text: translated, detected_source_language: "zh", target_language: "en" }] }; },
  };
  const store = new TranslationStore({ play: async () => {}, stop: () => {} }, 0, services);
  store.setConfig(config); store.setClip(clip);
  const root = createRoot(host);
  const renderReact = () => flushSync(() => root.render(React.createElement(TranslationPanel, { store, imageView })));
  renderReact();
  await createContentRenderers({ contentEl: content, badgeEl: element(app, "#preview-type-badge"),
    metaEl: element(app, "#preview-meta"), getLibraries: () => ({}), imageTranslation: imageView,
    services: { getClipImage: async () => png, getConfig: async () => config, ocrAvailable: async () => true,
      ocrImage: async () => recognized, copyText: async () => {}, openImageViewer: async () => { throw new Error("Viewer navigation is not part of this layout fixture"); }, fetchUrlMeta: async () => ({ url: "", title: null, description: null, favicon: null, site_name: null }) },
  }).renderImage(clip);
  // renderer 的 OCR 合同是非阻塞；只有已完成本地 promise 的微任务，无 timer 或外部 I/O。
  for (let i = 0; i < 8; i++) await Promise.resolve();
  const ocr = element<HTMLElement>(content, ".preview-ocr-result");
  const source = element<HTMLElement>(ocr, ".preview-ocr-source");
  const image = element<HTMLImageElement>(content, ".preview-image-open img");
  // 等 img 自己的 load 事件，而不是 decode()：decode() 在自己的任务里 resolve，可能排在
  // 文档 load 事件之后，而 --headless --screenshot 正是在文档 load 落盘，那一帧还没画出
  // 结论，脚本只能读到中性底色。img 的 load 必定早于文档 load（文档 load 要等这张图），
  // 之后剩下的 await 全是微任务，会在同一个任务的微任务检查点跑完，结论一定先画上。
  if (!image.complete) {
    await new Promise<void>((resolve, reject) => {
      image.addEventListener("load", () => resolve(), { once: true });
      image.addEventListener("error", () => reject(new Error("synthetic long image failed to load")), { once: true });
    });
  }
  assert(image.naturalWidth === 640 && image.naturalHeight === 1800, "synthetic long image did not decode");
  assert(element(source, "pre").textContent === recognized, "real OCR renderer did not publish the complete source");
  assert(!content.querySelector(".preview-code-scan"), "removed sidebar scan UI returned");
  assert(host.childElementCount === 0, "image translation duplicated below OCR");
  assert(getComputedStyle(scroll).overflowY === "auto", "image scroll owner is not preview-scroll");
  assert(getComputedStyle(content).overflowY === "visible", "preview-content creates a nested scroll");

  function verifyUnclipped(node: HTMLElement, name: string): void {
    assert(getComputedStyle(node).overflowY === "visible", `${name} creates a nested scroll`);
    assert(getComputedStyle(node).maxHeight === "none", `${name} is height-capped`);
    assert(node.scrollHeight <= node.clientHeight + 1, `${name} is internally clipped`);
  }
  function verifyTail(): void {
    assert(scroll.scrollHeight > scroll.clientHeight, "long image document does not scroll");
    scroll.scrollTop = scroll.scrollHeight;
    assert(ocr.getBoundingClientRect().bottom <= scroll.getBoundingClientRect().bottom + 1, "OCR/translation tail is unreachable");
  }
  verifyUnclipped(ocr, "OCR"); verifyTail();
  assert(image.getBoundingClientRect().bottom <= ocr.getBoundingClientRect().top + 1, "image overlaps OCR");
  // 已批准的侧栏缩略图上限为156px，144px是前一版布局的陈旧期望。
  const imageHeight = image.getBoundingClientRect().height;
  assert(imageHeight > 0 && imageHeight <= 156 + 1, `long image thumbnail is not compact: ${imageHeight}`);

  flushSync(() => { assert(revealImageTranslation(preview, imageView), "OCR translation slot failed to open"); });
  assert(requests === 0, "opening OCR translation sent an implicit request");
  const slot = element<HTMLElement>(ocr, ".preview-ocr-translation");
  const panel = element<HTMLElement>(slot, ".translation-panel");
  assert(source.hidden && !slot.hidden, "source and translation are shown together");
  assert(host.childElementCount === 0 && panel.parentElement === slot, "React panel is outside its OCR slot");
  await store.translate(); renderReact();
  const text = element<HTMLElement>(panel, ".translation-result-text");
  assert(text.textContent === translated, "translation lost its tail");
  verifyUnclipped(slot, "translation slot"); verifyUnclipped(panel, "translation panel");
  verifyUnclipped(text, "translation text"); verifyTail();
  assert(document.querySelectorAll("#translation-title").length === 0, "image translation duplicated a text heading id");
  flushSync(() => element<HTMLButtonElement>(ocr, ".preview-ocr-back").click());
  assert(!source.hidden && slot.hidden && slot.childElementCount === 0, "Back did not restore OCR and unmount the portal");
  assert(document.getElementById("translation-react-root") === host, "stable root was replaced");
  verifyTail();
  flushSync(() => { imageView.clear(); store.clear(); root.unmount(); });
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

/** 鼠标目标由真实浏览器命中测试选择，不能用 button.click 掩盖 pointer-events 穿透。 */
async function verifyExpandedMoreButtonHitTarget(app: HTMLElement): Promise<void> {
  const host = element<HTMLElement>(app, "#clipboard-react-root");
  host.replaceChildren();
  const root = createRoot(host);
  // 隔离审查可重放修前的单条CSS规则，证明此几何回归确实能捕获穿透。
  const legacyStyle = document.createElement("style");
  if (new URLSearchParams(location.search).has("legacyMore")) {
    legacyStyle.textContent = ".clip-row.expanded .clip-row-trigger { pointer-events: none; }";
    document.head.append(legacyStyle);
  }
  const clip: ClipItem = { id: 42, content_type: "text", text_content: "Synthetic hit-test row",
    html_content: null, image_data: null, content_hash: "fixture", is_favorite: false,
    is_sensitive: false, created_at: 1, byte_size: 22 };
  let expanded = false, toggles = 0, copies = 0;
  const handlers = {
    onFocus: () => {},
    onToggle: () => { expanded = !expanded; toggles++; render(); },
    onAction: () => { copies++; },
  };
  function render(): void {
    flushSync(() => root.render(React.createElement("main", { className: "clip-list" },
      React.createElement(ClipboardRow, { clip, index: 0, focused: true, focusedAction: -1,
        expanded, favoriteMode: false, locale: "en", handlers }))));
    // content-visibility:auto 的离屏优化需先paint；测命中时等价于已进入视口的行。
    // 只关闭这项布局优化，不改按钮pointer-events/层叠/几何等待测属性。
    element<HTMLElement>(host, ".clip-row").style.contentVisibility = "visible";
  }
  function clickMoreByPosition(): void {
    const button = element<HTMLButtonElement>(host, ".clip-row-trigger");
    const rect = button.getBoundingClientRect();
    const x = rect.left + rect.width / 2, y = rect.top + rect.height / 2;
    const hit = document.elementFromPoint(x, y);
    assert(hit === button || button.contains(hit), `More hit target escaped button while expanded=${expanded}: ${hit?.className}`);
    hit!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, clientX: x, clientY: y }));
  }
  try {
    render(); clickMoreByPosition();
    assert(expanded && toggles === 1, "first More click did not expand actions");
    clickMoreByPosition();
    assert(!expanded && toggles === 2 && copies === 0, "second More click copied the row instead of collapsing");
    for (const button of host.querySelectorAll<HTMLButtonElement>(".clip-row-action")) {
      assert(button.disabled && button.tabIndex === -1, "collapsed action remains keyboard actionable");
    }
  } finally { flushSync(() => root.unmount()); legacyStyle.remove(); }
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

async function run(): Promise<void> {
try {
  const app = mountProductLayout();
  verifyPreviewAndTranslationShareTheColumn(app);
  await verifyImageAndOcrUseOneScroller(app);
  verifyCodecPanelKeepsListWidth(app);
  verifyVirtualRowHeights(app);
  await verifyExpandedMoreButtonHitTarget(app);
  document.documentElement.dataset.layoutSmoke = "passed";
  paint("#00d000");
} catch (error) {
  document.documentElement.dataset.layoutSmoke = "failed";
  document.body.title = String(error);
  console.error(String(error));
  paint("#d00000", String(error));
}

}
void run();
