// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import Markdown from './Markdown';
import { MediaContext, MediaList } from './Media';
import { ResourceMenu } from './ResourceMenu';
import { call } from './api';

vi.mock('./api', () => ({ desktop: true, call: vi.fn(async () => ({ token: 'full', previewToken: 'small' })) }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (token: string) => `http://supercode-media.localhost/${token}` }));
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('IntersectionObserver', undefined);
  vi.mocked(call).mockClear();
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

it('groups six Markdown images into one gallery and keeps an opened image stable while streaming', async () => {
  const images = Array.from({length:6}, (_,i) => `![图片${i+1}](<D:/output/兔子 ${i+1}.png>)`);
  await act(async () => root.render(<Markdown text={images[0]} streaming/>));
  const first = container.querySelector('img');
  await act(async () => container.querySelector<HTMLButtonElement>('.chat-image-button')!.click());
  const preview = document.querySelector('.image-preview img');
  for (let i=2;i<=6;i++) await act(async () => root.render(<Markdown text={images.slice(0,i).join('\n\n')} streaming/>));
  await act(async () => root.render(<Markdown text={images.join('\n\n')}/>));
  expect(container.querySelectorAll('.chat-image-gallery')).toHaveLength(1);
  expect(container.querySelector('.chat-image-gallery')?.getAttribute('data-image-count')).toBe('6');
  expect(container.querySelectorAll('.chat-image-gallery img')).toHaveLength(6);
  expect(container.querySelector('img')).toBe(first);
  expect(document.querySelector('.image-preview img')).toBe(preview);
  expect(container.querySelectorAll('p .chat-media-card')).toHaveLength(0);
});

it('preserves prose, code examples and video boundaries between image groups', async () => {
  const text = '![一](one.png) ![二](two.png)\n\n说明文字。\n\n![三](three.png)\n\n![视频](clip.mp4)\n\n```md\n![示例](example.png)\n```';
  await act(async () => root.render(<Markdown text={text}/>));
  expect([...container.querySelectorAll('.chat-image-gallery')].map(node=>node.getAttribute('data-image-count'))).toEqual(['2','1']);
  expect(container.querySelectorAll('.chat-image-gallery .chat-video-start')).toHaveLength(0);
  expect(container.querySelectorAll('.chat-media-card')).toHaveLength(4);
  expect(container.textContent).toContain('说明文字。');
  expect(container.querySelector('pre')?.textContent).toContain('![示例](example.png)');
});

it('uses the same compact gallery for tool-returned media without mixing audio into it', async () => {
  await act(async () => root.render(<MediaList media={[
    {path:'one.png',name:'一',kind:'image'}, {path:'two.png',name:'二',kind:'image'},
    {path:'voice.wav',name:'音频',kind:'audio'}, {path:'three.png',name:'三',kind:'image'},
  ]}/>));
  expect([...container.querySelectorAll('.chat-image-gallery')].map(node=>node.getAttribute('data-image-count'))).toEqual(['2','1']);
  expect(container.querySelector('.chat-image-gallery audio')).toBeNull();
  expect(container.querySelector('audio')?.preload).toBe('none');
});

async function resourceImage(path: string, context: {projectId?:string;sessionId?:string} = {}) {
  const openPath = vi.fn();
  await act(async () => root.render(<MediaContext.Provider value={context}><ResourceMenu openFile={vi.fn()} openPath={openPath}><Markdown text={`![图片](<${path}>)`}/></ResourceMenu></MediaContext.Provider>));
  return openPath;
}
async function rightClick(image: HTMLElement) {
  await act(async () => image.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:20,clientY:20})));
}
function folderItem() { return [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(node=>node.textContent==='在文件夹中显示')!; }

it('reveals a relative image in its chat workspace through the media route, including files outside projects', async () => {
  const openPath = await resourceImage('兔子 图1.png',{sessionId:'chat-files'});
  await rightClick(container.querySelector('img')!);
  expect(folderItem().disabled).toBe(false);
  await act(async () => folderItem().click());
  expect(call).toHaveBeenCalledWith('reveal_media',{path:'兔子 图1.png',projectId:null,sessionId:'chat-files'});
  expect(openPath).not.toHaveBeenCalled();
  expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(container.querySelector('img')).not.toBeNull();
});

it('supports right-clicking an enlarged image and lets Escape close the menu before the preview', async () => {
  await resourceImage('D:/Downloads/兔子 图片.png',{projectId:'p'});
  const button = container.querySelector<HTMLButtonElement>('.chat-image-button')!;
  await act(async () => button.click());
  await rightClick(document.querySelector('.image-preview img')!);
  expect(folderItem()).toBeDefined();
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  await rightClick(document.querySelector('.image-preview img')!);
  await act(async () => folderItem().click());
  expect(call).toHaveBeenCalledWith('reveal_media',{path:'D:/Downloads/兔子 图片.png',projectId:'p',sessionId:null});
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(button);
});

it('reports reveal failures without breaking the image preview', async () => {
  await resourceImage('D:/Downloads/兔子.png');
  const image = container.querySelector('img');
  await rightClick(image!);
  vi.mocked(call).mockRejectedValueOnce(new Error('文件不存在'));
  await act(async () => folderItem().click());
  expect(document.querySelector('[role="menu"] [role="alert"]')?.textContent).toContain('文件不存在');
  expect(container.querySelector('img')).toBe(image);
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
});

it('does not offer a local folder for remote-only images', async () => {
  await resourceImage('https://example.com/bunny.png');
  await rightClick(container.querySelector('img')!);
  expect(folderItem()).toBeUndefined();
  expect(document.querySelector('[role="menu"]')?.textContent).toContain('复制链接');
  expect(call).not.toHaveBeenCalledWith('reveal_media',expect.anything());
});
