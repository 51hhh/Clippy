import { beforeEach, afterEach, expect, it, vi } from 'vitest';
vi.mock('../js/api.ts', () => ({ getClips: vi.fn(), deleteClip: vi.fn(), selectClip: vi.fn(), toggleFavorite: vi.fn(), copyText: vi.fn(), speakClip: vi.fn(), speakText: vi.fn(), translateClip: vi.fn(), translationHistory: vi.fn() }));
import * as api from '../js/api.ts';
import { ClipboardStore } from '../react/main/clipboardStore.ts';
import { TranslationStore } from '../react/main/translationStore.ts';
import { createKeyboardRouter } from '../js/keyboard-router.js';
const clip = (id, text = `item ${id}`) => ({ id, content_type: 'text', text_content: text, html_content: null, image_data: null, content_hash: String(id), is_favorite: false, is_sensitive: false, created_at: id, byte_size: text.length });
function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
const config = { translation_services: [{ provider: 'libretranslate', enabled: true, endpoint: 'https://example.test' }], translation_target_language: 'zh' };
const batch = { request_id: 1, services: [{ status: 'ok', provider: 'libretranslate', translated_text: '你好', detected_source_language: 'en', target_language: 'zh' }] };
const spoken = { mime_type: 'audio/mpeg', audio_base64: 'SUQz' };
function translator() { const player = { play: vi.fn().mockResolvedValue(undefined), stop: vi.fn() }; const store = new TranslationStore(player); store.setConfig(config); store.setClip(clip(1)); return { store, player }; }
beforeEach(() => { vi.resetAllMocks(); api.getClips.mockResolvedValue([]); api.translationHistory.mockResolvedValue([]); document.body.replaceChildren(); });
afterEach(() => vi.useRealTimers());
it('收起 Delete 后 Enter 只复制行本体', async () => {
    const store = new ClipboardStore();
    api.getClips.mockResolvedValue([clip(1)]);
    await store.refresh();
    store.expandRowActions();
    store.moveCol(1);
    store.moveCol(1);
    store.collapseActions();
    await store.activateFocus();
    expect(api.deleteClip).not.toHaveBeenCalled();
    expect(api.selectClip).toHaveBeenCalledWith(1);
});
it.each(['preview-content', 'update-modal'])('保留 %s 按钮的 Enter/Space 原生激活', rootId => {
    const root = document.createElement('div');
    root.id = rootId;
    const button = document.createElement('button');
    root.append(button);
    document.body.append(root);
    const activateFocus = vi.fn();
    const router = createKeyboardRouter({ clipboardList: { search: { isVisible: () => false }, activateFocus }, previewPanel: {}, codec: {}, pinClip: vi.fn(), hidePanel: vi.fn() });
    for (const key of ['Enter', ' ']) {
        const event = { key, target: button, preventDefault: vi.fn() };
        router.onKeyDown(event);
        expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(activateFocus).not.toHaveBeenCalled();
});
it('输入框、可编辑内容和文本选区不驱动背景列表', () => {
    const moveRow = vi.fn(), selectByIndex = vi.fn(), activateFocus = vi.fn();
    const router = createKeyboardRouter({ clipboardList: { search: { isVisible: () => false }, moveRow, selectByIndex, activateFocus }, previewPanel: {}, codec: {}, pinClip: vi.fn(), hidePanel: vi.fn() });
    for (const tag of ['input', 'textarea', 'select', 'div']) {
        const target = document.createElement(tag);
        if (tag === 'div')
            target.setAttribute('contenteditable', 'true');
        document.body.append(target);
        for (const key of ['s', '3', ' ']) {
            const event = { key, target, preventDefault: vi.fn() };
            router.onKeyDown(event);
            expect(event.preventDefault).not.toHaveBeenCalled();
        }
    }
    const text = document.createTextNode('selected text');
    document.body.append(text);
    const range = document.createRange();
    range.selectNodeContents(text);
    window.getSelection().addRange(range);
    router.onKeyDown({ key: 's', target: document.body, preventDefault: vi.fn() });
    window.getSelection().removeAllRanges();
    expect(moveRow).not.toHaveBeenCalled();
    expect(selectByIndex).not.toHaveBeenCalled();
    expect(activateFocus).not.toHaveBeenCalled();
});
it('弹窗打开且焦点仍在背景时列表不执行动作', () => {
    const modal = document.createElement('div');
    modal.id = 'update-modal';
    document.body.append(modal);
    const activateFocus = vi.fn();
    const router = createKeyboardRouter({ clipboardList: { search: { isVisible: () => false }, activateFocus }, previewPanel: {}, codec: {}, pinClip: vi.fn(), hidePanel: vi.fn() });
    router.onKeyDown({ key: 'Enter', target: document.body, preventDefault: vi.fn() });
    expect(activateFocus).not.toHaveBeenCalled();
});
it('搜索结果遇新增事件时按原查询重新读取', async () => {
    const store = new ClipboardStore();
    api.getClips.mockResolvedValue([clip(1, 'needle')]);
    await store.setQuery('needle');
    store.prependClip(clip(2, 'unrelated'));
    expect(store.getSnapshot().all.map(c => c.id)).toEqual([1]);
    await vi.waitFor(() => expect(api.getClips).toHaveBeenCalledTimes(2));
    expect(api.getClips).toHaveBeenLastCalledWith('needle', false, 0, 30);
});
it('搜索未完成切收藏再回全部时重新获取当前查询', async () => {
    const store = new ClipboardStore();
    api.getClips.mockResolvedValueOnce([clip(1, 'old')]);
    await store.refresh();
    const search = deferred();
    api.getClips.mockReturnValueOnce(search.promise).mockResolvedValueOnce([]).mockResolvedValueOnce([clip(2, 'needle')]);
    const query = store.setQuery('needle');
    await store.setPanelMode('favorites');
    search.resolve([clip(9, 'stale')]);
    await query;
    await store.setPanelMode('all');
    expect(store.getSnapshot().query).toBe('needle');
    expect(store.getSnapshot().all.map(c => c.id)).toEqual([2]);
});
it('删除使旧刷新失效，补查完成前不能清 dirty 或复活行', async () => {
    const store = new ClipboardStore();
    api.getClips.mockResolvedValueOnce([clip(1), clip(2)]);
    await store.refresh();
    const stale = deferred(), fresh = deferred();
    api.getClips.mockReturnValueOnce(stale.promise).mockReturnValueOnce(fresh.promise);
    const refreshing = store.refresh();
    store.removeClip(1);
    stale.resolve([clip(1), clip(2)]);
    await refreshing;
    expect(store.getSnapshot().all.map(c => c.id)).toEqual([2]);
    expect(store.isDirty()).toBe(true);
    fresh.resolve([clip(2)]);
    await vi.waitFor(() => expect(store.isDirty()).toBe(false));
    expect(store.getSnapshot().all.map(c => c.id)).toEqual([2]);
});
it.each(['insert', 'delete'])('分页中 %s 后从零补查已加载范围，再继续分页不重不漏', async (operation) => {
    let rows = Array.from({ length: 90 }, (_, i) => clip(90 - i));
    const store = new ClipboardStore();
    api.getClips.mockImplementation(async (_, __, offset, limit) => rows.slice(offset, offset + limit));
    await store.refresh();
    await store.loadMore();
    store.focusRow(35);
    const selected = store.getFocusedClip().id;
    const stale = deferred();
    api.getClips.mockReturnValueOnce(stale.promise);
    const loading = store.loadMore();
    if (operation === 'insert') {
        rows.unshift(clip(91));
        store.prependClip(clip(91));
    }
    else {
        rows = rows.filter(c => c.id !== 89);
        store.removeClip(89);
    }
    stale.resolve(rows.slice(60, 90));
    await loading;
    await vi.waitFor(() => expect(store.isDirty()).toBe(false));
    expect(store.getSnapshot().all.map(c => c.id)).toEqual(rows.slice(0, 60).map(c => c.id));
    expect(store.getFocusedClip().id).toBe(selected);
    await store.loadMore();
    expect(store.getSnapshot().all.map(c => c.id)).toEqual(rows.slice(0, 90).map(c => c.id));
});
it('隐藏后新增事件保持缓存释放，重新打开才读取首屏', async () => {
    const store = new ClipboardStore();
    api.getClips.mockResolvedValue([clip(1)]);
    await store.refresh();
    store.releaseMemory();
    store.prependClip(clip(2));
    await Promise.resolve();
    expect(store.getSnapshot().all).toEqual([]);
    expect(api.getClips).toHaveBeenCalledTimes(1);
    api.getClips.mockResolvedValue([clip(2), clip(1)]);
    await store.refresh();
    expect(store.getFocusedClip().id).toBe(2);
});
it('相同条目焦点通知不取消在途翻译', async () => {
    const list = new ClipboardStore();
    const { store } = translator();
    list.initialize({ onFocusChange: c => store.setClip(c) });
    api.getClips.mockResolvedValue([clip(1)]);
    await list.refresh();
    const request = deferred();
    api.translateClip.mockReturnValue(request.promise);
    const translating = store.translate();
    list.prependClip(clip(2));
    expect(list.getFocusedClip().id).toBe(1);
    expect(store.getSnapshot().loading).toBe(true);
    request.resolve(batch);
    await translating;
    expect(store.getSnapshot().cards[0].translatedText).toBe('你好');
});
it('翻译与朗读独立执行，翻译不能令朗读永久 busy', async () => {
    const { store, player } = translator();
    const audio = deferred();
    api.speakClip.mockReturnValue(audio.promise);
    api.translateClip.mockResolvedValue(batch);
    const speaking = store.speakSource();
    await store.translate();
    audio.resolve(spoken);
    await speaking;
    expect(player.play).toHaveBeenCalled();
    expect(store.getSnapshot().speaking).toBeNull();
});
it('旧条目朗读失败不能清除新条目正在朗读的状态', async () => {
    const { store } = translator();
    const oldAudio = deferred(), newAudio = deferred();
    api.speakClip.mockReturnValueOnce(oldAudio.promise).mockReturnValueOnce(newAudio.promise);
    const oldSpeaking = store.speakSource();
    store.setClip(clip(2));
    const newSpeaking = store.speakSource();
    oldAudio.reject(new Error('translation.network: failed'));
    await oldSpeaking;
    expect(store.getSnapshot().speaking).toBe('source');
    expect(store.getSnapshot().speechErrorCode).toBeNull();
    newAudio.resolve(spoken);
    await newSpeaking;
    expect(store.getSnapshot().speaking).toBeNull();
});
it('切换收藏后另一个面板缓存失效，不显示过期收藏标记', async () => {
    const store = new ClipboardStore();
    let favorite = false;
    api.getClips.mockImplementation(async (_, onlyFavorites) => onlyFavorites && !favorite ? [] : [{ ...clip(1), is_favorite: favorite }]);
    await store.refresh();
    await store.setPanelMode('favorites');
    await store.setPanelMode('all');
    api.toggleFavorite.mockImplementation(async () => { favorite = true; });
    await store.invokeAction(store.getFocusedClip(), 'favorite');
    await store.setPanelMode('favorites');
    expect(store.getSnapshot().favorites.map(c => c.id)).toEqual([1]);
});
it('隐藏预览后翻译与朗读完成都不能恢复旧内容', async () => {
    const { store, player } = translator();
    store.setPanelVisible(true);
    const audio = deferred(), translation = deferred();
    api.speakClip.mockReturnValue(audio.promise);
    api.translateClip.mockReturnValue(translation.promise);
    const speaking = store.speakSource(), translating = store.translate();
    store.setPanelVisible(false);
    audio.resolve(spoken);
    translation.resolve(batch);
    await Promise.all([speaking, translating]);
    expect(player.play).not.toHaveBeenCalled();
    expect(store.getSnapshot()).toMatchObject({ cards: [], loading: false, speaking: null });
});
