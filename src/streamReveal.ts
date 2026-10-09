import type { Element, Root, RootContent, Text } from 'hast';

export const revealDuration = 700;
const segmentDelay = 24;
const maxDelay = 160;
const maxRanges = 128;
const segmenter = new Intl.Segmenter('zh', { granularity: 'word' });
interface RevealRange { start: number; end: number; at: number }

// Keep only the small, still fading tail. The reply itself is never queued or shortened.
export class StreamReveal {
  text = '';
  private live = false;
  private ranges: RevealRange[] = [];
  private nextAt = 0;

  update(text: string, streaming: boolean, now: number) {
    this.ranges = this.ranges.filter(range => range.at + revealDuration > now);
    if (text !== this.text) {
      if (text.startsWith(this.text) && (streaming || this.live)) {
        const start = this.text.length;
        const delta = text.slice(start);
        for (const segment of segmenter.segment(delta)) {
          if (!segment.segment.trim()) continue;
          const at = Math.min(Math.max(now, this.nextAt), now + maxDelay);
          this.ranges.push({ start: start + segment.index, end: start + segment.index + segment.segment.length, at });
          this.nextAt = at + segmentDelay;
        }
        this.ranges = this.ranges.slice(-maxRanges);
      } else {
        this.ranges = [];
        this.nextAt = now;
      }
      this.text = text;
    }
    this.live = streaming;
  }

  get expiresAt() { return this.ranges.reduce((latest, range) => Math.max(latest, range.at + revealDuration), 0); }
  get activeCount() { return this.ranges.length; }

  decorate(root: Root) {
    if (!this.ranges.length) return;
    const first = this.ranges[0].start;
    const visit = (parent: Root | Element) => {
      const children: RootContent[] = [];
      for (const node of parent.children) {
        if (node.type === 'element') {
          // Code arrives immediately and keeps normal selection, shaping and copying.
          if (!['pre', 'code'].includes(node.tagName)) visit(node);
          children.push(node);
        } else if (node.type === 'text' && (node.position?.end.offset ?? 0) > first) {
          children.push(...this.fragments(node));
        } else children.push(node);
      }
      parent.children = children as Element['children'];
    };
    visit(root);
  }

  private fragments(node: Text): RootContent[] {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    // Entities and escaped syntax may change their rendered length: leave them intact.
    if (start == null || end == null || this.text.slice(start, end) !== node.value) return [node];
    const result: RootContent[] = [];
    let cursor = start;
    for (const range of this.ranges) {
      const from = Math.max(start, range.start);
      const to = Math.min(end, range.end);
      if (from >= to) continue;
      if (from > cursor) result.push({ type: 'text', value: node.value.slice(cursor - start, from - start) });
      result.push({ type: 'element', tagName: 'span', properties: {
        className: ['stream-text-fragment'], 'data-stream-at': range.at,
        'data-stream-key': `${from}:${to}`,
      }, children: [{ type: 'text', value: node.value.slice(from - start, to - start) }] });
      cursor = to;
    }
    if (cursor < end) result.push({ type: 'text', value: node.value.slice(cursor - start) });
    return result.length ? result : [node];
  }
}
