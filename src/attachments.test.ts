import { describe, expect, it } from 'vitest';
import { clipboardImages, imageBytes, MAX_IMAGE_BYTES, mergeAttachments, validateImageFile } from './attachments';
import { decodeDrafts, encodeDrafts } from './workspaceState';

describe('image attachments', () => {
  it('extracts clipboard images without intercepting ordinary text', () => {
    const image = { type: 'image/png', name: 'image.png' } as File;
    const textFile = { type: 'text/plain', name: 'notes.txt' } as File;
    const item = (file: File) => ({ kind: 'file', getAsFile: () => file }) as DataTransferItem;
    expect(clipboardImages({ items: [item(image), item(textFile)] as unknown as DataTransferItemList, files: [] as unknown as FileList })).toEqual([image]);
    expect(clipboardImages({ items: [] as unknown as DataTransferItemList, files: [image] as unknown as FileList })).toEqual([image]);
    expect(clipboardImages({ items: [{ kind: 'string', type: 'text/plain' }] as unknown as DataTransferItemList, files: [] as unknown as FileList })).toEqual([]);
  });
  it('rejects oversized and unsupported images before reading their contents', () => {
    expect(() => validateImageFile({ name: 'large.png', type: 'image/png', size: MAX_IMAGE_BYTES + 1 })).toThrow('10 MB');
    expect(() => validateImageFile({ name: 'vector.svg', type: 'image/svg+xml', size: 100 })).toThrow('仅支持');
    expect(() => validateImageFile({ name: 'empty.png', type: 'image/png', size: 0 })).toThrow('为空');
    expect(() => validateImageFile({ name: '截图.PNG', type: '', size: 100 })).not.toThrow();
  });
  it('deduplicates stored pictures and enforces the combined image budget', () => {
    const image = { kind: 'image' as const, name: '截图.png', path: 'cache/a.png', size: MAX_IMAGE_BYTES };
    expect(mergeAttachments([image], [image])).toEqual([image]);
    const two = mergeAttachments([image], [{ ...image, path: 'cache/b.png' }]);
    expect(imageBytes(two)).toBe(20 * 1024 * 1024);
    expect(() => mergeAttachments(two, [{ ...image, path: 'cache/c.png', size: 1 }])).toThrow('20 MB');
    expect(() => mergeAttachments(Array.from({ length: 12 }, (_, i) => ({ kind: 'file', name: String(i), path: String(i) })), [image])).toThrow('12 个');
  });
  it('restores image-only drafts with names and size metadata without image bytes in localStorage', () => {
    const image = { kind: 'image' as const, name: '截图.png', path: 'cache/a.png', size: 100 };
    const serialized = encodeDrafts(new Map(), new Map([['project:claude', [image]]]));
    expect(decodeDrafts(serialized).get('project:claude')?.attachments).toEqual([image]);
    expect(serialized).not.toContain('base64');
  });
});
