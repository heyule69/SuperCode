import { describeLink } from './links';
import type { Message } from './types';

export type MediaKind = 'image' | 'video' | 'audio';
export interface MediaReference { path: string; name: string; kind: MediaKind }

export function mediaKind(source: string): MediaKind | null {
  let path = source;
  try { if (/^https?:/i.test(source)) path = new URL(source).pathname; } catch { return null; }
  if (/\.(png|jpe?g|gif|webp|svg|bmp)$/i.test(path)) return 'image';
  if (/\.(mp4|m4v|webm|mov|mkv)$/i.test(path)) return 'video';
  if (/\.(mp3|wav|ogg|oga|flac|m4a)$/i.test(path)) return 'audio';
  return null;
}

export function mediaReference(source: string, name?: string, kind?: MediaKind): MediaReference | null {
  if (!source || source.length > 8192) return null;
  const link = describeLink(source);
  if (!['file', 'external'].includes(link.kind) || link.kind === 'external' && !/^https?:/i.test(link.target)) return null;
  const type = kind ?? mediaKind(link.target);
  if (!type || /^data:|^blob:|^javascript:/i.test(source)) return null;
  const filename = link.target.split(/[\\/]/).pop()?.split(/[?#]/)[0];
  return { path: link.target, kind: type, name: name?.trim().slice(0, 160) || filename || (type === 'image' ? '图片' : type === 'video' ? '视频' : '音频') };
}

export function messageMedia(message: Message): MediaReference[] {
  const refs = Array.isArray(message.data?.media) ? message.data.media : [];
  const result = refs.slice(0, 12).flatMap((value: unknown) => {
    if (!value || typeof value !== 'object') return [];
    const item = value as Record<string, unknown>;
    if (typeof item.path !== 'string' || !['image', 'video', 'audio'].includes(String(item.kind))) return [];
    const ref = mediaReference(item.path, typeof item.name === 'string' ? item.name : undefined, item.kind as MediaKind);
    return ref ? [ref] : [];
  });
  if (message.kind === 'imageView' && !result.length) { const ref = mediaReference(String(message.data?.path ?? message.text), undefined, 'image'); if (ref) result.push(ref); }
  return [...new Map(result.map(ref => [ref.path, ref])).values()];
}

// Completed links and standalone code paths can also identify existing outputs.
// A fenced list containing only absolute media paths is an output list; other
// code blocks are examples and must not cause media loads.
export function referencedMedia(text: string): MediaReference[] {
  const pathLists = Array.from(text.matchAll(/(?:```|~~~)[^\n]*\n([^]*?)\n(?:```|~~~)/g)).flatMap(match => {
    const lines = match[1].trim().split('\n').map(line => line.trim());
    if (!lines.length || lines.length > 12 || !lines.every(line => /^(?:[a-z]:[\\/]|\/(?!\/))/i.test(line))) return [];
    const media = lines.map(line => mediaReference(line));
    return media.every(ref => ref !== null) ? media as MediaReference[] : [];
  });
  const prose = text.replace(/```[^]*?(?:```|$)|~~~[^]*?(?:~~~|$)/g, '');
  const embedded = new Set(Array.from(prose.matchAll(/!\[[^\]]*\]\(\s*(?:<([^>]+)>|([^\s)]+))[^)]*\)/g), m => m[1] ?? m[2]));
  const refs: MediaReference[] = pathLists;
  for (const match of prose.matchAll(/(?<!!)\[([^\]]*)\]\(\s*(?:<([^>]+)>|([^\s)]+))(?:\s+"[^"]*")?\s*\)|`([^`\n]+)`/g)) {
    const source = match[2] ?? match[3] ?? match[4];
    if (embedded.has(source)) continue;
    if (match[4] && /^(?:ffmpeg|ffprobe|convert|magick|python|node|curl|wget|copy|cp|mv|del|rm|start|npm|npx|git|powershell|cmd)\s|&&|\|/i.test(source)) continue;
    const ref = mediaReference(source, match[1]);
    if (ref) refs.push(ref);
  }
  return [...new Map(refs.map(ref => [ref.path, ref])).values()].slice(0, 12);
}
