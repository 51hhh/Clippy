import { beforeEach, describe, expect, it, vi } from 'vitest';
import { classifyText } from '../js/preview/classify.js';
import { MAX_RENDER_CHARS } from '../js/preview/large-text.js';
import { createCodeRenderers } from '../js/preview/code-renderers.js';
import { numberBaseInfo } from '../js/preview/format-detectors.js';
import { isJson, isBase64, isJwt } from '../js/preview/detectors.js';
let contentEl, badgeEl, highlight, renderers;
beforeEach(() => {
    contentEl = document.createElement('div');
    badgeEl = document.createElement('span');
    highlight = vi.fn().mockReturnValue({ value: '' });
    renderers = createCodeRenderers({ contentEl, badgeEl, getLibraries: () => ({ hljs: { highlight }, DOMPurify: { sanitize: x => x } }) });
});
it('预算之前不解析超限 JSON 或解码超限 Base64/JWT', () => {
    const json = JSON.stringify({ body: 'x'.repeat(MAX_RENDER_CHARS) });
    const parse = vi.spyOn(JSON, 'parse');
    expect(classifyText(json)).toBeNull();
    expect(isJson(json)).toBe(false);
    expect(parse).not.toHaveBeenCalled();
    parse.mockRestore();
    const base64 = btoa('a'.repeat(MAX_RENDER_CHARS));
    const decode = vi.spyOn(globalThis, 'atob');
    expect(classifyText(base64)).toBeNull();
    expect(isBase64(base64)).toBe(false);
    expect(isJwt('eyJ.' + base64 + '.abc')).toBe(false);
    expect(decode).not.toHaveBeenCalled();
    decode.mockRestore();
});
it('超限 JSON 直接调用渲染器也只展示有说明的原文片段', () => {
    const json = JSON.stringify({ body: 'x'.repeat(MAX_RENDER_CHARS) });
    renderers.renderJson(json);
    expect(highlight).not.toHaveBeenCalled();
    expect(contentEl.querySelector('pre').textContent).toHaveLength(MAX_RENDER_CHARS);
    expect(contentEl.querySelector('.preview-truncated')).not.toBeNull();
    expect(badgeEl.textContent).toBe('TEXT');
});
it('深层 JSON/JWT 格式展开超过预算时不完整展开、不高亮', () => {
    const json = '['.repeat(1000) + '0' + ']'.repeat(1000);
    renderers.renderJson(json);
    expect(highlight).not.toHaveBeenCalled();
    expect(contentEl.querySelector('pre').textContent).toBe(json);
    expect(contentEl.querySelector('.preview-truncated')).not.toBeNull();
    contentEl.replaceChildren();
    renderers.renderJwt(btoa('{"alg":"HS256"}') + '.' + btoa(json) + '.abc');
    expect(highlight).not.toHaveBeenCalled();
    expect(contentEl.querySelector('.preview-truncated')).not.toBeNull();
});
it('普通 JSON 仍格式化与高亮，边界不多截一个字符', () => {
    renderers.renderJson('{"answer":42}');
    expect(highlight).toHaveBeenCalledWith('{\n  "answer": 42\n}', { language: 'json' });
    contentEl.replaceChildren();
    const exact = JSON.stringify({ a: 'x'.repeat(MAX_RENDER_CHARS - 8) });
    expect(exact.length).toBe(MAX_RENDER_CHARS);
    renderers.renderJson(exact);
    expect(contentEl.querySelector('pre').textContent).toBe(exact);
});
it('解码结果也独立受预算约束，不能撑爆 DOM', () => {
    renderers.renderEncoded({ type: 'base64', original: 'fixture', decoded: 'x'.repeat(MAX_RENDER_CHARS + 1) });
    expect(contentEl.querySelector('pre').textContent).toBe('fixture');
    expect(contentEl.querySelector('.preview-truncated')).not.toBeNull();
});
describe('整数进制转换精度', () => {
    it.each(['9007199254740991', '9007199254740992', '9007199254740993', '9223372036854775807', '18446744073709551615', '-9223372036854775808', '00000123', '-0x20000000000001', '+0o777', '0b1010', '-0b1010'])('%s 保留精确整数', value => {
        const sign = value.startsWith('-') ? -1n : 1n;
        const unsigned = value.replace(/^[+-]/, '');
        const expected = sign * BigInt(unsigned);
        const actual = numberBaseInfo(value);
        expect(actual.decimal).toBe(expected);
        const prefix = expected < 0 ? '-' : '';
        const absolute = expected < 0 ? -expected : expected;
        expect(actual.hex).toBe(`${prefix}0x${absolute.toString(16).toUpperCase()}`);
        expect(actual.binary).toBe(`${prefix}0b${absolute.toString(2)}`);
    });
    it.each(['0x', '0b102', '0o8', '12.3', '3e4', '--1', '12junk', '', 'NaN'])('明确拒绝不合法整数 %s', value => expect(numberBaseInfo(value)).toBeNull());
});
