import { expect, it } from 'vitest';
import { mediaReference, messageMedia, referencedMedia } from './mediaSources';
import { mergeEvent } from './events';

it('supports local, project and remote media while rejecting executable sources and credentials', () => {
  expect(mediaReference('file:///D:/work/%E6%88%AA%E5%9B%BE.png')?.path).toBe('D:/work/截图.png');
  expect(mediaReference('renders/demo.mp4')?.kind).toBe('video');
  expect(mediaReference('https://example.com/demo.webm?token=123')?.kind).toBe('video');
  expect(mediaReference('https://example.com/screenshot', '截图', 'image')?.kind).toBe('image');
  for (const source of ['javascript:alert(1)', 'data:image/png;base64,AAAA', 'blob:test', 'https://user:pass@example.com/p.png', 'cmd:run', 'hello.txt']) expect(mediaReference(source)).toBeNull();
});

it('recognizes output references without duplicating embeds or treating code examples as media', () => {
  const text = '![截图](<D:/work/截图.png>)\n\n[视频](<D:/work/demo.mp4>) · `D:/work/demo.mp4` · `audio.wav`\n\n```sh\n![范例](example.png)\n`sample.mp4`\n```\n\n`ffmpeg -i input.mov output.mp4`';
  expect(referencedMedia(text).map(ref => ref.path)).toEqual(['D:/work/demo.mp4', 'audio.wav']);
  expect(referencedMedia('```\n[范例](example.png)')).toEqual([]);
  expect(referencedMedia('文件位置：\n```\nC:\\Users\\Alice\\Desktop\\截图.png\nC:\\Users\\Alice\\Desktop\\视频.mp4\n```').map(ref => ref.kind)).toEqual(['image', 'video']);
});

it('preserves normalized screenshot references when large tool diagnostics are bounded', () => {
  const media = [{ type: 'media', kind: 'image', path: 'D:/cache/截图.png', name: '截图' }];
  const item = { id: 'shot', type: 'mcpToolCall', status: 'completed', arguments: Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`part${i}`, 'x'.repeat(32768)])), media };
  const [message] = mergeEvent([], { method: 'item/completed', params: { turnId: 't', item } }, 's');
  expect(messageMedia(message)).toEqual([{ kind: 'image', path: 'D:/cache/截图.png', name: '截图' }]);
  expect(JSON.stringify(message.data).length).toBeLessThan(140 * 1024);
});
