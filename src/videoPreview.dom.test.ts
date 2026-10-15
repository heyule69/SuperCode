// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { firstVideoFrame } from './videoPreview';

let videos: HTMLVideoElement[];
let draw: ReturnType<typeof vi.fn>;
beforeEach(() => {
  vi.useFakeTimers();
  videos = [];
  draw = vi.fn();
  const create = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation(((tag: string, options?: ElementCreationOptions) => {
    const node = create(tag, options);
    if (tag === 'video') {
      const video = node as HTMLVideoElement;
      Object.defineProperties(video, { videoWidth: { value: 1920 }, videoHeight: { value: 1080 } });
      vi.spyOn(video, 'load').mockImplementation(() => {});
      vi.spyOn(video, 'pause').mockImplementation(() => {});
      videos.push(video);
    }
    if (tag === 'canvas') {
      vi.spyOn(node as HTMLCanvasElement, 'getContext').mockReturnValue({ drawImage: draw } as unknown as CanvasRenderingContext2D);
      vi.spyOn(node as HTMLCanvasElement, 'toDataURL').mockReturnValue('data:image/jpeg;base64,ZnJhbWU=');
    }
    return node;
  }) as typeof document.createElement);
});
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); });

it('captures the initial frame without playing and releases its video source after decoding', async () => {
  const result = firstVideoFrame('https://example.com/clip.mp4', new AbortController().signal);
  await Promise.resolve();
  const video = videos[0];
  expect(video.muted).toBe(true);
  expect(video.autoplay).toBe(false);
  expect(video.currentTime).toBe(0);
  video.dispatchEvent(new Event('loadeddata'));
  expect(await result).toContain('data:image/jpeg');
  expect(draw).toHaveBeenCalledWith(video, 0, 0, 960, 540);
  expect(video.hasAttribute('src')).toBe(false);
  expect(video.pause).toHaveBeenCalled();
  expect(video.load).toHaveBeenCalledTimes(2);
  expect(vi.getTimerCount()).toBe(0);
});

it('serializes decoders and skips a queued card after it unmounts', async () => {
  const first = firstVideoFrame('https://example.com/one.mp4', new AbortController().signal);
  const controller = new AbortController();
  const second = firstVideoFrame('https://example.com/two.mp4', controller.signal);
  const rejected = expect(second).rejects.toMatchObject({ name: 'AbortError' });
  await Promise.resolve();
  expect(videos).toHaveLength(1);
  controller.abort();
  videos[0].dispatchEvent(new Event('loadeddata'));
  await first;
  await rejected;
  expect(videos).toHaveLength(1);
});

it('releases a failed or timed-out decoder so another preview can start', async () => {
  const first = firstVideoFrame('https://example.com/broken.mp4', new AbortController().signal);
  const failure = expect(first).rejects.toThrow('首帧');
  await Promise.resolve();
  videos[0].dispatchEvent(new Event('error'));
  await failure;
  expect(videos[0].hasAttribute('src')).toBe(false);
  const second = firstVideoFrame('https://example.com/stalled.mp4', new AbortController().signal);
  const timeout = expect(second).rejects.toThrow('超时');
  await vi.advanceTimersByTimeAsync(8000);
  await timeout;
  expect(videos[1].hasAttribute('src')).toBe(false);
  expect(vi.getTimerCount()).toBe(0);
});

it('cancels an active preview without leaving a decoder or a late result behind', async () => {
  const controller = new AbortController();
  const result = firstVideoFrame('https://example.com/clip.mp4', controller.signal);
  const rejected = expect(result).rejects.toMatchObject({ name: 'AbortError' });
  await Promise.resolve();
  controller.abort();
  videos[0].dispatchEvent(new Event('loadeddata'));
  await rejected;
  expect(draw).not.toHaveBeenCalled();
  expect(videos[0].hasAttribute('src')).toBe(false);
  expect(vi.getTimerCount()).toBe(0);
});
