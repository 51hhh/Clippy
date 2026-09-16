import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { init } from "../i18n/i18n.js";
import { createContentRenderers } from "../js/preview/content-renderers.js";
import { createImageTranslationView, revealImageTranslation } from "../js/preview/reveal-translation.js";
import { createKeyboardRouter, resolveKeyboardMode } from "../js/keyboard-router.js";
import { TranslationPanel } from "../react/main/TranslationPanel.tsx";
import { TranslationStore } from "../react/main/translationStore.ts";

const config = {
  ocr_enabled: true, ocr_result_mode: "preview", translation_target_language: "en",
  translation_services: [{ provider: "libretranslate", enabled: true, endpoint: "https://example.test/translate" }],
};
const entry = (id, type = "image", sensitive = false) => ({
  id, content_type: type, is_sensitive: sensitive, text_content: `text ${id}`, byte_size: 42,
});
function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
const result = text => ({ request_id: 1, services: [{ status: "ok", provider: "libretranslate", translated_text: text, target_language: "en" }] });

describe("OCR translation portal lifecycle", () => {
  let root, host, content, panel, imageView, store, services, generation, selected, listActions;
  beforeEach(async () => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    init("en");
    document.body.innerHTML = '<div id="list-panel" tabindex="-1"></div><section class="preview-panel"><div class="preview-scroll"><div id="preview-content"></div><div id="translation-react-root"></div></div></section>';
    content = document.getElementById("preview-content");
    host = document.getElementById("translation-react-root");
    panel = document.querySelector(".preview-panel");
    imageView = createImageTranslationView();
    generation = 0;
    listActions = { moveRow: vi.fn(), selectByIndex: vi.fn(), activateFocus: vi.fn() };
    services = {
      getClipImage: vi.fn(async () => "image"), getConfig: vi.fn(async () => config),
      ocrAvailable: vi.fn(async () => true), ocrImage: vi.fn(async id => `recognized ${id}`),
      openImageViewer: vi.fn(async () => "viewer"), copyText: vi.fn(async () => {}),
      translateClip: vi.fn(async () => result("translated")), translationHistory: vi.fn(async () => []),
      speakClip: vi.fn(), speakText: vi.fn(),
    };
    store = new TranslationStore({ stop() {}, play: async () => {} }, 0, services);
    store.setConfig(config);
    store.setPanelVisible(true);
    root = createRoot(host);
    await act(async () => root.render(React.createElement(TranslationPanel, { store, imageView })));
  });
  afterEach(async () => {
    await act(async () => { imageView.clear(); store.clear(); root.unmount(); });
    document.body.replaceChildren();
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  });

  function select(clip) {
    selected = clip;
    const current = ++generation;
    imageView.clear();
    // 与生产一致：只清空 vanilla 内容，React 根始终留在独立宿主中。
    content.replaceChildren();
    store.setClip(clip);
    if (clip.content_type !== "image") { content.textContent = clip.text_content; return Promise.resolve(); }
    return createContentRenderers({ contentEl: content, badgeEl: document.createElement("span"),
      metaEl: document.createElement("span"), getLibraries: () => ({}), services, imageTranslation: imageView,
    }).renderImage(clip, () => current === generation);
  }
  function keyboard() {
    return createKeyboardRouter({
      clipboardList: { ...listActions, getFocusedClip: () => selected, search: { isVisible: () => false } },
      previewPanel: { isVisible: () => true, revealTranslation: () => revealImageTranslation(panel, imageView) },
      codec: {}, pinClip: vi.fn(), hidePanel() {}, translation: store,
    });
  }
  function key(key, target = document.getElementById("list-panel"), extra = {}) {
    return { key, target, preventDefault: vi.fn(), ...extra };
  }

  it("opens inside OCR only after the slow slot exists, then preserves results when returning to source", async () => {
    const image = deferred();
    services.getClipImage.mockReturnValueOnce(image.promise);
    let pending;
    await act(async () => { pending = select(entry(1)); });
    const focus = document.getElementById("list-panel"); focus.focus();
    const slowTab = key("Tab", focus, { shiftKey: true });
    await act(async () => { keyboard().onKeyDown(slowTab); keyboard().onKeyDown(key("Enter", focus, { ctrlKey: true })); });
    expect(slowTab.preventDefault).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(focus);
    expect(services.translateClip).not.toHaveBeenCalled();
    await act(async () => { image.resolve("ready"); await pending; });
    expect(host.childElementCount).toBe(0);
    expect(content.querySelector(".translation-panel")).toBeNull();
    await act(async () => content.querySelector(".preview-ocr-translate").click());
    const slot = content.querySelector(".preview-ocr-translation");
    expect(slot.querySelector(".translation-panel")).not.toBeNull();
    expect(host.childElementCount).toBe(0);
    expect(content.querySelector(".preview-ocr-source").hidden).toBe(true);
    expect(services.translateClip).not.toHaveBeenCalled();
    await act(async () => slot.querySelector(".translation-action").click());
    expect(slot.textContent).toContain("translated");
    await act(async () => content.querySelector(".preview-ocr-back").click());
    expect(slot.childElementCount).toBe(0);
    expect(content.querySelector(".preview-ocr-source").textContent).toContain("recognized 1");
    expect(document.activeElement).toBe(content.querySelector(".preview-ocr-translate"));
    await act(async () => content.querySelector(".preview-ocr-translate").click());
    expect(slot.textContent).toContain("translated");
    expect(services.translateClip).toHaveBeenCalledTimes(1);
  });

  it("keeps the stable root through image A → slow image B → text and discards old work without changing focus", async () => {
    await act(async () => { await select(entry(1)); revealImageTranslation(panel, imageView); });
    const oldSlot = content.querySelector(".preview-ocr-translation");
    const translation = deferred(); services.translateClip.mockReturnValueOnce(translation.promise);
    let pendingTranslation;
    await act(async () => { pendingTranslation = store.translate(); });
    const image = deferred(); services.getClipImage.mockReturnValueOnce(image.promise);
    let pendingImage;
    await act(async () => { pendingImage = select(entry(2)); });
    expect(oldSlot.childElementCount).toBe(0);
    expect(imageView.getSnapshot()).toBeNull();
    const list = document.getElementById("list-panel"); list.focus();
    await act(async () => {
      await select(entry(3, "text")); image.resolve("late B"); translation.resolve(result("late A"));
      await pendingImage; await pendingTranslation;
    });
    expect(document.getElementById("translation-react-root")).toBe(host);
    expect(host.querySelector("#translation-title")).not.toBeNull();
    expect(content.textContent).toBe("text 3");
    expect(document.body.textContent).not.toContain("late A");
    expect(document.activeElement).toBe(list);
    await act(async () => { await select(entry(4)); });
    expect(imageView.getSnapshot()).toMatchObject({ clipId: 4, visible: false });
    expect(host.childElementCount).toBe(0);
  });

  it.each(["hide preview", "clear main window"])("unmounts an active portal on %s and late responses cannot restore it", async operation => {
    await act(async () => { await select(entry(1)); revealImageTranslation(panel, imageView); });
    const oldSlot = content.querySelector(".preview-ocr-translation");
    const request = deferred(); services.translateClip.mockReturnValueOnce(request.promise);
    let pending;
    await act(async () => { pending = store.translate(); });
    const list = document.getElementById("list-panel"); list.focus();
    await act(async () => {
      generation++; imageView.clear();
      if (operation === "hide preview") store.setPanelVisible(false);
      else { content.replaceChildren(); store.clear(); }
    });
    expect(oldSlot.childElementCount).toBe(0);
    await act(async () => { request.resolve(result("late hidden result")); await pending; });
    expect(document.querySelector(".translation-panel")).toBeNull();
    expect(document.activeElement).toBe(list);
  });

  it("routes portaled controls and Back to the translation keyboard mode while sensitive shortcuts stay local", async () => {
    await act(async () => { await select(entry(1, "image", true)); });
    await act(async () => keyboard().onKeyDown(key("Tab", undefined, { shiftKey: true })));
    const back = content.querySelector(".preview-ocr-back");
    expect(document.activeElement).toBe(back);
    expect(resolveKeyboardMode({ target: back })).toBe("translation");
    const action = content.querySelector(".translation-action");
    expect(action.disabled).toBe(true);
    expect(resolveKeyboardMode({ target: action })).toBe("translation");
    await act(async () => {
      keyboard().onKeyDown(key("Enter", back, { ctrlKey: true }));
      keyboard().onKeyDown(key("Escape", back));
    });
    expect(services.translateClip).not.toHaveBeenCalled();
    expect(services.speakClip).not.toHaveBeenCalled();
    expect(document.activeElement.id).toBe("list-panel");
    await act(async () => keyboard().onKeyDown(key("Enter", undefined, { ctrlKey: true })));
    expect(services.translateClip).not.toHaveBeenCalled();
  });

  it("keeps original and restored OCR text navigation local, and explicitly translates from the original view", async () => {
    await act(async () => { await select(entry(1)); });
    const source = content.querySelector(".preview-ocr-source pre");
    const assertNativeReading = () => {
      source.focus();
      for (const name of ["ArrowDown", "ArrowUp", "1", "0", "Enter", "s", "w"]) {
        const event = key(name, source); keyboard().onKeyDown(event);
        expect(event.preventDefault).not.toHaveBeenCalled();
      }
      expect(listActions.moveRow).not.toHaveBeenCalled();
      expect(listActions.selectByIndex).not.toHaveBeenCalled();
      expect(listActions.activateFocus).not.toHaveBeenCalled();
    };
    assertNativeReading();
    await act(async () => keyboard().onKeyDown(key("Enter", source, { ctrlKey: true })));
    expect(content.querySelector(".preview-ocr-translation").textContent).toContain("translated");
    expect(services.translateClip).toHaveBeenCalledTimes(1);
    await act(async () => content.querySelector(".preview-ocr-back").click());
    assertNativeReading();
    keyboard().onKeyDown(key("Escape", source));
    expect(document.activeElement.id).toBe("list-panel");
  });
});
