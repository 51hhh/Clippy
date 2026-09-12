import { beforeEach, afterEach, expect, it, vi } from 'vitest';
vi.mock('../js/api.ts', () => ({ checkUpdate: vi.fn(), downloadAndInstallUpdate: vi.fn(), getAppVersion: vi.fn(), getInstallType: vi.fn(), openExternalUrl: vi.fn(), restartApp: vi.fn() }));
import * as api from '../js/api.ts';
import * as i18n from '../i18n/i18n.js';
import { createThemePicker } from '../js/settings/theme-picker.js';
import { createAutostartSettings } from '../js/settings/autostart-settings.js';
function mount() {
    const modal = document.createElement('div');
    modal.id = 'update-modal';
    modal.className = 'hidden';
    for (const id of ['update-title', 'update-version', 'update-body', 'update-progress', 'update-progress-bar', 'update-progress-text', 'update-btn-skip', 'update-btn-later', 'update-btn-install', 'update-btn-close', 'update-btn-download']) {
        const el = document.createElement(id.includes('btn') ? 'button' : 'div');
        el.id = id;
        modal.append(el);
    }
    document.body.append(modal);
    return modal;
}
beforeEach(() => { vi.resetAllMocks(); document.body.replaceChildren(); i18n.init('en'); api.checkUpdate.mockResolvedValue({ available: true, version: '9.9.9', body: 'Fixture', update: {} }); api.getInstallType.mockResolvedValue('appimage'); });
afterEach(() => vi.resetModules());
it.each(['appimage', 'macos'])('%s 安装完成后有稍后和显式重启入口', async (type) => {
    const modal = mount();
    api.getInstallType.mockResolvedValue(type);
    const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js');
    initUpdateModal();
    await checkForUpdate(true);
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.installedTitle')));
    expect(document.getElementById('update-btn-later').classList.contains('hidden')).toBe(false);
    expect(document.getElementById('update-btn-install').classList.contains('hidden')).toBe(false);
    expect(api.restartApp).not.toHaveBeenCalled();
    document.getElementById('update-btn-later').click();
    expect(modal.classList.contains('hidden')).toBe(true);
    await checkForUpdate(true);
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(api.restartApp).toHaveBeenCalledTimes(1));
    expect(api.downloadAndInstallUpdate).toHaveBeenCalledTimes(1);
});
it('Windows 交给安装器，不在旧进程提供错误重启路径', async () => {
    mount();
    api.getInstallType.mockResolvedValue('windows');
    const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js');
    initUpdateModal();
    await checkForUpdate(true);
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.installerTitle')));
    expect(document.getElementById('update-btn-close').classList.contains('hidden')).toBe(false);
    expect(document.getElementById('update-btn-install').classList.contains('hidden')).toBe(true);
    expect(api.restartApp).not.toHaveBeenCalled();
});
it('安装失败显示准确失败说明并提供关闭和手动下载', async () => {
    mount();
    api.downloadAndInstallUpdate.mockRejectedValue(new Error('signature invalid'));
    const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js');
    initUpdateModal();
    await checkForUpdate(true);
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.failedTitle')));
    expect(document.getElementById('update-body').textContent).toBe(i18n.t('update.failedBody'));
    expect(document.getElementById('update-btn-close').classList.contains('hidden')).toBe(false);
});
it('手动检查失败必须告知调用方，不能被当成已是最新版', async () => {
    mount();
    api.checkUpdate.mockRejectedValue(new Error('offline'));
    const { checkForUpdate } = await import('../js/update-modal.js');
    await expect(checkForUpdate(true)).rejects.toThrow('offline');
});
it('主题保存失败恢复已保存主题并通知用户', async () => {
    const container = document.createElement('div');
    document.body.append(container);
    const notify = vi.fn();
    const picker = createThemePicker({ container, translate: key => key, persistTheme: vi.fn().mockRejectedValue(new Error('disk full')), notify });
    picker.initialize('dark');
    container.querySelector('[data-theme="rose"]').click();
    await vi.waitFor(() => expect(picker.value).toBe('dark'));
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(notify).toHaveBeenCalled();
});
it('重启失败后仍可重试或稍后退出', async () => {
    mount();
    api.restartApp.mockRejectedValue(new Error('restart unavailable'));
    const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js');
    initUpdateModal();
    await checkForUpdate(true);
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.installedTitle')));
    document.getElementById('update-btn-install').click();
    await vi.waitFor(() => expect(document.getElementById('update-body').textContent).toBe(i18n.t('update.restartFailed')));
    expect(document.getElementById('update-btn-install').disabled).toBe(false);
    expect(document.getElementById('update-btn-later').classList.contains('hidden')).toBe(false);
});
it('下载中的双击和再次检查不能发起重复安装', async () => {
    mount();
    let finish;
    api.downloadAndInstallUpdate.mockReturnValue(new Promise(resolve => { finish = resolve; }));
    const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js');
    initUpdateModal();
    initUpdateModal();
    await checkForUpdate(true);
    const button = document.getElementById('update-btn-install');
    button.click();
    button.click();
    await checkForUpdate(true);
    expect(api.downloadAndInstallUpdate).toHaveBeenCalledTimes(1);
    expect(api.checkUpdate).toHaveBeenCalledTimes(1);
    finish();
    await vi.waitFor(() => expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.installedTitle')));
    i18n.init('zh-CN');
    expect(document.getElementById('update-btn-install').textContent).toBe(i18n.t('update.restart'));
    expect(document.getElementById('update-title').textContent).toBe(i18n.t('update.installedTitle'));
});
it('设置写入按顺序读取最新配置，保留 tmux 与内部字段', async () => {
    const { createConfigWriter } = await import('../js/settings/config-writer.js');
    let current = { theme: 'light', tmux_capture: false, capture_probe_hint_shown: true, main_window_position: { x: 42, y: 21 }, unknown_future_field: 'preserved' };
    let releaseTheme;
    const themeWait = new Promise(resolve => { releaseTheme = resolve; });
    const updateConfig = vi.fn(async (config) => { if (config.theme === 'dark' && !config.tmux_capture)
        await themeWait; current = config; return { shortcut_status: 'pending' }; });
    const onSaved = vi.fn();
    const writer = createConfigWriter({ getConfig: async () => ({ ...current }), updateConfig, onSaved });
    const theme = writer.write({ theme: 'dark' });
    const tmux = writer.run(async () => { current = { ...current, tmux_capture: true }; });
    const save = writer.write({ max_history: 99 });
    releaseTheme();
    await theme;
    await tmux;
    await expect(save).resolves.toEqual({ shortcut_status: 'pending' });
    expect(current).toMatchObject({ theme: 'dark', tmux_capture: true, capture_probe_hint_shown: true, main_window_position: { x: 42, y: 21 }, unknown_future_field: 'preserved', max_history: 99 });
    expect(onSaved).toHaveBeenCalledTimes(2);
});
it('设置保存失败不会更新成功状态，也不会阻塞用户修正后重试', async () => {
    const { createConfigWriter } = await import('../js/settings/config-writer.js');
    const onSaved = vi.fn();
    const updateConfig = vi.fn().mockRejectedValueOnce(new Error('disk full')).mockResolvedValueOnce({ shortcut_status: 'unchanged' });
    const writer = createConfigWriter({ getConfig: async () => ({ theme: 'light' }), updateConfig, onSaved });
    await expect(writer.write({ max_history: 7 })).rejects.toThrow('disk full');
    expect(onSaved).not.toHaveBeenCalled();
    await writer.write({ max_history: 8 });
    expect(onSaved).toHaveBeenCalledOnce();
    expect(onSaved).toHaveBeenCalledWith({ theme: 'light', max_history: 8 });
});
it('更新弹窗接收焦点且 Tab 不跳到后台控件', async () => {
    const background = document.createElement('button'); document.body.append(background); background.focus();
    const modal = mount(); const { initUpdateModal, checkForUpdate } = await import('../js/update-modal.js'); initUpdateModal(); await checkForUpdate(true);
    expect(document.activeElement).toBe(document.getElementById('update-btn-install'));
    const event = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }); document.activeElement.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true); expect(document.activeElement).toBe(document.getElementById('update-btn-skip'));
    document.activeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(modal.classList.contains('hidden')).toBe(true); expect(document.activeElement).toBe(background);
});

function autostartFixture(development) {
    const group = document.createElement('div');
    const row = document.createElement('div'); row.className = 'setting-toggle-row';
    const toggle = document.createElement('input'); toggle.type = 'checkbox';
    row.append(toggle); group.append(row); document.body.append(group);
    const dependencies = { toggle, isDevBinary: vi.fn().mockResolvedValue(development), isEnabled: vi.fn().mockResolvedValue(true), enable: vi.fn(), disable: vi.fn(), translate: i18n.t, notify: vi.fn() };
    return { ...dependencies, controller: createAutostartSettings(dependencies) };
}
it('开发版设置不读取或删除共享启动项，禁用控件阻止覆写正式版', async () => {
    const fixture = autostartFixture(true);
    await fixture.controller.load();
    expect(fixture.toggle.disabled).toBe(true);
    expect(fixture.isEnabled).not.toHaveBeenCalled();
    fixture.toggle.click();
    fixture.toggle.dispatchEvent(new Event('change'));
    expect(fixture.enable).not.toHaveBeenCalled();
    expect(fixture.disable).not.toHaveBeenCalled();
    const hint = document.querySelector('.autostart-dev-hint');
    expect(hint.textContent).toBe(i18n.t('settings.autostart.devHint'));
    i18n.init('zh-CN');
    expect(hint.textContent).toBe(i18n.t('settings.autostart.devHint'));
});
it('运行身份读取失败时保留自启动项且保持不可修改', async () => {
    const fixture = autostartFixture(false);
    fixture.isDevBinary.mockRejectedValue(new Error('IPC unavailable'));
    await fixture.controller.load();
    fixture.toggle.dispatchEvent(new Event('change'));
    expect(fixture.toggle.disabled).toBe(true);
    expect(fixture.isEnabled).not.toHaveBeenCalled();
    expect(fixture.disable).not.toHaveBeenCalled();
});
it('正式版读取自启动状态不写入；用户切换失败会还原并提示', async () => {
    const fixture = autostartFixture(false);
    fixture.disable.mockRejectedValue(new Error('permission denied'));
    await fixture.controller.load();
    expect(fixture.toggle.checked).toBe(true);
    expect(fixture.enable).not.toHaveBeenCalled();
    expect(fixture.disable).not.toHaveBeenCalled();
    fixture.toggle.click();
    await vi.waitFor(() => expect(fixture.notify).toHaveBeenCalledOnce());
    expect(fixture.disable).toHaveBeenCalledOnce();
    expect(fixture.toggle.disabled).toBe(false);
    expect(fixture.toggle.checked).toBe(true);
});
