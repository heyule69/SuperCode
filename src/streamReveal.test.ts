import { describe, expect, it } from 'vitest';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkRehype from 'remark-rehype';
import type { Element, Root, RootContent } from 'hast';
import { revealDuration, StreamReveal } from './streamReveal';

const parser = unified().use(remarkParse).use(remarkRehype);
const parse = (text: string) => parser.runSync(parser.parse(text)) as Root;
function textOf(node: Root | RootContent): string { return node.type === 'text' ? node.value : 'children' in node ? node.children.map(textOf).join('') : ''; }
function fragments(node: Root | RootContent): Element[] {
  return 'children' in node ? [...(node.type === 'element' && node.properties['data-stream-key'] ? [node] : []), ...node.children.flatMap(fragments)] : [];
}

describe('streamed Markdown reveal', () => {
  it('keeps loaded history immediately readable', () => {
    const reveal = new StreamReveal();
    reveal.update('**已有回复**，保持稳定。', false, 100);
    const root = parse(reveal.text);
    reveal.decorate(root);
    expect(fragments(root)).toHaveLength(0);
    expect(textOf(root)).toBe('已有回复，保持稳定。');
  });

  it('continues earlier fade times across new text and Markdown structure changes', () => {
    const reveal = new StreamReveal();
    reveal.update('已完成。\n\n正在**调整', true, 100);
    const before = parse(reveal.text);
    reveal.decorate(before);
    const firstAt = fragments(before)[0].properties['data-stream-at'];
    reveal.update('已完成。\n\n正在**调整界面**，继续输出。', true, 180);
    const after = parse(reveal.text);
    const expected = textOf(after);
    reveal.decorate(after);
    expect(textOf(after)).toBe(expected);
    expect(fragments(after)[0].properties['data-stream-at']).toBe(firstAt);
    expect(fragments(after).some(span => Number(span.properties['data-stream-at']) >= 180)).toBe(true);
    reveal.update(reveal.text, false, 200);
    const completed = parse(reveal.text);
    reveal.decorate(completed);
    expect(fragments(completed)[0].properties['data-stream-at']).toBe(firstAt);
  });

  it('preserves Unicode, escaped text, entities, links and code contents', () => {
    const reveal = new StreamReveal();
    const text = '中文 👨‍👩‍👧‍👦 é &amp; \\*文字 [文件](./a.ts)\n\n```ts\nconst value = "你好";\n```';
    reveal.update(text, true, 0);
    const root = parse(text);
    const expected = textOf(root);
    reveal.decorate(root);
    expect(textOf(root)).toBe(expected);
    const pre = root.children.find(node => node.type === 'element' && node.tagName === 'pre');
    expect(pre && fragments(pre)).toHaveLength(0);
  });

  it('bounds animation state while preserving a large burst in full', () => {
    const reveal = new StreamReveal();
    reveal.update('连续输出。'.repeat(3000), true, 100);
    const root = parse(reveal.text);
    reveal.decorate(root);
    expect(reveal.activeCount).toBeLessThanOrEqual(128);
    expect(fragments(root).length).toBeLessThanOrEqual(128);
    expect(textOf(root)).toBe(reveal.text);
    expect(reveal.expiresAt).toBeLessThanOrEqual(100 + 160 + revealDuration);
    reveal.update(reveal.text, false, reveal.expiresAt + 1);
    const settled = parse(reveal.text);
    reveal.decorate(settled);
    expect(fragments(settled)).toHaveLength(0);
    expect(reveal.activeCount).toBe(0);
  });

  it('shows a corrected or shortened response without replaying it', () => {
    const reveal = new StreamReveal();
    reveal.update('原来的流式回复', true, 0);
    reveal.update('替换后的完整回复', false, 100);
    const root = parse(reveal.text);
    reveal.decorate(root);
    expect(textOf(root)).toBe('替换后的完整回复');
    expect(fragments(root)).toHaveLength(0);
  });
});
