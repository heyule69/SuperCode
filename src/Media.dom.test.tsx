// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import Markdown from './Markdown';
import { MediaContext } from './Media';
import { call } from './api';

vi.mock('./api', () => ({ desktop: true, call: vi.fn(async () => ({ token: 'full', previewToken: 'small' })) }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (token: string) => `http://supercode-media.localhost/${token}` }));
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('IntersectionObserver', undefined);
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

it('loads thumbnails, opens the original on one click, and closes with Escape', async () => {
  await act(async () => root.render(<MediaContext.Provider value={{ projectId: 'p' }}><Markdown text={'![截图](<D:/output/截图.png>)'}/></MediaContext.Provider>));
  const img = container.querySelector('img')!;
  expect(img.src).toContain('/small'); expect(img.loading).toBe('lazy');
  expect(call).toHaveBeenCalledWith('prepare_media', { path: 'D:/output/截图.png', projectId: 'p', sessionId: null });
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="在系统中打开 截图"]')!.click());
  expect(call).toHaveBeenCalledWith('open_media', { path: 'D:/output/截图.png', projectId: 'p', sessionId: null });
  const button = container.querySelector<HTMLButtonElement>('.chat-image-button')!;
  await act(async () => button.click());
  expect(document.querySelector('.image-preview img')?.getAttribute('src')).toContain('/full');
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })));
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(button);
});

it('keeps images and an opened video player mounted across streaming deltas and completion', async () => {
  const prefix = '![截图](<D:/output/截图.png>)\n\n![视频](<D:/output/demo.mp4>)\n\n';
  await act(async () => root.render(<Markdown text={prefix + '第一段。'} streaming/>));
  await act(async () => container.querySelector<HTMLButtonElement>('.chat-video-start')!.click());
  const image = container.querySelector('img'), video = container.querySelector('video')!;
  expect(video.controls).toBe(true); expect(video.preload).toBe('metadata'); expect(video.autoplay).toBe(false);
  for (let i = 0; i < 5; i++) await act(async () => root.render(<Markdown text={prefix + `第${i}段。`} streaming/>));
  await act(async () => root.render(<Markdown text={prefix + '完成。'}/>));
  expect(container.querySelector('img')).toBe(image); expect(container.querySelector('video')).toBe(video);
});

it('does not preload audio or automatically play it', async () => {
  await act(async () => root.render(<Markdown text={'![音频](<D:/output/demo.wav>)'}/>));
  const audio = container.querySelector('audio')!;
  expect(audio.controls).toBe(true); expect(audio.preload).toBe('none'); expect(audio.autoplay).toBe(false);
});


it('resolves relative generated images against a projectless chat workspace', async () => {
  await act(async () => root.render(<MediaContext.Provider value={{ sessionId: 'chat-files' }}><Markdown text="![生成图片](result.png)"/></MediaContext.Provider>));
  expect(call).toHaveBeenCalledWith('prepare_media', { path:'result.png', projectId:null, sessionId:'chat-files' });
});
