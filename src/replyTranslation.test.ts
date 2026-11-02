import { describe, expect, it } from 'vitest';
import { maskTranslation, needsTranslation, restoreTranslation, translationParts, validateTranslation } from './replyTranslation';

describe('display translation preparation', () => {
  it('detects English and other languages while skipping Chinese prose, code and product identifiers', () => {
    for (const text of ['Numbering is updated to 19 sections, about 1150 characters.', 'Build passed.', 'Build passed', 'Je continue', 'Done.', 'こんにちは、完了しました。', '작업을 완료했습니다.', 'Я обновлю заголовок.', 'سأقوم بتحديث العنوان.']) expect(needsTranslation(text), text).toBe(true);
    for (const text of ['已完成布局优化，下一步更新标题。', '使用 React 和 TypeScript 完成页面。', 'OpenAI Codex', '`npm run build`', '```js\nconst english = "hello world";\n```', 'https://example.com/demo', 'D:\\outputs\\hello.png']) expect(needsTranslation(text), text).toBe(false);
  });
  it('protects executable code, destinations, URLs, images and reference definitions byte for byte', () => {
    const original = 'Please read [the report](D:/输出/报告.md) and `npm run build`.\n\n```ts\nconst title = "hello world";\n```\n\n![preview](https://example.com/a.png)\n\nhttps://example.com/a?token=secret\n\n[manual]: https://docs.example.com\n\nD:\\outputs\\file.txt';
    const prepared = maskTranslation(original);
    expect(prepared.masked).not.toContain('secret');
    expect(prepared.masked).not.toContain('const title');
    expect(prepared.masked).not.toContain('D:/输出');
    expect(restoreTranslation(prepared.masked, prepared.values)).toBe(original);
    const translated = prepared.masked.replace('Please read', '请阅读').replace('the report', '报告').replace('and', '并运行');
    expect(restoreTranslation(validateTranslation(prepared.masked, translated), prepared.values)).toContain('```ts\nconst title = "hello world";\n```');
  });
  it('only translates stable streaming paragraphs, never splits tokens, and round trips long text', () => {
    const prepared = maskTranslation('I have updated the header.\n\nNext I will update the structure');
    const parts = translationParts(prepared.masked, true);
    expect(parts.filter(p => p.translate).map(p => p.text)).toEqual(['I have updated the header.']);
    expect(parts.at(-1)).toEqual({ text:'Next I will update the structure', translate:false });
    expect(translationParts('Still streaming without paragraph', true)).toEqual([{ text:'Still streaming without paragraph', translate:false }]);
    const long = maskTranslation(('Here is the next paragraph with `important code`. '.repeat(300)) + '\n\n最后一段保持中文。');
    const chunks = translationParts(long.masked, false);
    expect(chunks.length).toBeGreaterThan(3);
    expect(chunks.map(p => p.text).join('')).toBe(long.masked);
    expect(chunks.every(p => new TextEncoder().encode(p.text).length <= 12288)).toBe(true);
    expect(restoreTranslation(chunks.map(p => p.text).join(''), long.values)).toContain('最后一段保持中文。');
  });
  it('recognizes a foreign paragraph inside a mostly Chinese reply', () => {
    const parts = translationParts(maskTranslation(`${'这一段是中文，无需重复翻译。'.repeat(30)}\n\nNext I will update the header.`).masked, false);
    expect(parts.filter(p => p.translate).map(p => p.text)).toEqual(['Next I will update the header.']);
  });
  it('rejects empty, untranslated, missing, duplicate, reordered and invented protected content', () => {
    const source = 'Read ⟪SC0⟫ then ⟪SC1⟫.';
    for (const bad of ['', null, 'Read ⟪SC0⟫ then ⟪SC1⟫.', '请阅读 ⟪SC0⟫', '请阅读 ⟪SC1⟫ ⟪SC0⟫', '请阅读 ⟪SC0⟫ ⟪SC0⟫ ⟪SC1⟫', '请阅读 ⟪SC0⟫ ⟪SC1⟫ https://new.example', '请阅读 ⟪SC0⟫ ⟪SC1⟫ `invented code`']) expect(() => validateTranslation(source, bad)).toThrow();
    expect(validateTranslation(source, '请阅读 ⟪SC0⟫，然后阅读 ⟪SC1⟫。')).toContain('请阅读');
  });
});
