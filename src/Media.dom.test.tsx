// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import Markdown from './Markdown';
import { MediaContext, MediaList, ProcessMedia } from './Media';
import { ResourceMenu } from './ResourceMenu';
import { call } from './api';
import { firstVideoFrame } from './videoPreview';

vi.mock('./api', () => ({ desktop: true, call: vi.fn(async () => ({ token: 'full', previewToken: 'small' })) }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (token: string) => `http://supercode-media.localhost/${token}` }));
vi.mock('./videoPreview', () => ({ firstVideoFrame: vi.fn(async () => 'data:image/jpeg;base64,cG9zdGVy') }));
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal('IntersectionObserver', undefined);
  vi.mocked(call).mockClear();
  vi.mocked(firstVideoFrame).mockClear();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });

it('prepares a Windows preview image instead of rejecting its Markdown-encoded path', async () => {
  const path='C:\\Users\\Administrator\\Documents\\Codex\\中文目录\\work\\beautify_data.png';
  await act(async()=>root.render(<Markdown text={`![公司信息页预览](<${path}>)`}/>));
  expect(container.textContent).not.toContain('无法显示此来源');
  expect(call).toHaveBeenCalledWith('prepare_media',expect.objectContaining({path}));
  expect(container.querySelector('.chat-image-button')).not.toBeNull();
});

it('loads process images only after a click and unmounts previews when folded again', async()=>{
  const media=[{path:'D:/cache/step.jpg',name:'过程截图',kind:'image' as const}];
  await act(async()=>root.render(<ProcessMedia media={media}/>));
  expect(container.textContent).toContain('已查看 1 张图像');
  expect(container.querySelector('img')).toBeNull();expect(call).not.toHaveBeenCalled();
  const toggle=container.querySelector<HTMLButtonElement>('.process-media-summary')!;
  await act(async()=>toggle.click());
  expect(call).toHaveBeenCalledWith('prepare_media',{path:'D:/cache/step.jpg',projectId:null,sessionId:null,processPreview:true});
  expect(container.querySelector('.chat-media-process img')).not.toBeNull();
  await act(async()=>container.querySelector<HTMLButtonElement>('.chat-image-button')!.click());
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  await act(async()=>toggle.click());
  expect(container.querySelector('img')).toBeNull();expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(container.querySelector('.chat-media-card')).toBeNull();
});

it('ignores a late preparation result after the user folds the process images',async()=>{
  let finish!:(value:unknown)=>void;
  vi.mocked(call).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
  await act(async()=>root.render(<ProcessMedia media={[{path:'D:/cache/late.jpg',name:'截图',kind:'image'}]}/>));
  const toggle=container.querySelector<HTMLButtonElement>('.process-media-summary')!;
  await act(async()=>toggle.click());await act(async()=>toggle.click());
  await act(async()=>finish({token:'late-full',previewToken:'late-small'}));
  expect(container.querySelector('img')).toBeNull();expect(container.querySelector('.chat-media-card')).toBeNull();
});

it('explains an expired process screenshot without a broken image or unusable file action',async()=>{
  vi.mocked(call).mockResolvedValueOnce({unavailable:'expired'});
  await act(async()=>root.render(<ProcessMedia media={[{path:'D:/cache/process-captures/old.jpg',name:'截图',kind:'image'}]}/>));
  await act(async()=>container.querySelector<HTMLButtonElement>('.process-media-summary')!.click());
  expect(container.textContent).toContain('过程截图已自动清理');expect(container.querySelector('img')).toBeNull();
  expect(container.querySelector('[aria-label^="在系统中打开"]')).toBeNull();
  expect(container.querySelector('.chat-media-card')?.hasAttribute('data-media-source')).toBe(false);
});

it('counts every process image but mounts at most one page, keeping the open page stable during new snapshots',async()=>{
  const media=Array.from({length:25},(_,index)=>({path:`D:/cache/${index}.jpg`,name:`截图${index}`,kind:'image' as const}));
  await act(async()=>root.render(<ProcessMedia media={media}/>));
  expect(container.textContent).toContain('已查看 25 张图像');expect(call).not.toHaveBeenCalled();
  await act(async()=>container.querySelector<HTMLButtonElement>('.process-media-summary')!.click());
  expect(container.querySelectorAll('.chat-media-card')).toHaveLength(12);
  expect(container.querySelector('[data-media-source="D:/cache/24.jpg"]')).not.toBeNull();
  await act(async()=>root.render(<ProcessMedia media={[...media,{path:'D:/cache/25.jpg',name:'截图25',kind:'image'}]}/>));
  expect(container.querySelector('[data-media-source="D:/cache/25.jpg"]')).toBeNull();
  await act(async()=>container.querySelector<HTMLButtonElement>('.process-media-pages button')!.click());
  expect(container.querySelectorAll('.chat-media-card')).toHaveLength(12);
  expect(container.querySelector('[data-media-source="D:/cache/12.jpg"]')).not.toBeNull();
  expect(container.querySelector('[data-media-source="D:/cache/24.jpg"]')).toBeNull();
});

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

it('shows a first-frame poster without mounting or automatically starting the full player', async () => {
  await act(async () => root.render(<Markdown text="![视频](clip.mp4)"/>));
  expect(firstVideoFrame).toHaveBeenCalledWith('http://supercode-media.localhost/full', expect.any(AbortSignal));
  expect(container.querySelector('video')).toBeNull();
  const poster = container.querySelector<HTMLImageElement>('.chat-video-poster')!;
  expect(poster.src).toBe('data:image/jpeg;base64,cG9zdGVy');
  expect(poster.alt).toContain('首帧');
  await act(async () => container.querySelector<HTMLButtonElement>('.chat-video-start')!.click());
  const player = container.querySelector('video')!;
  expect(player.getAttribute('poster')).toBe(poster.src);
  expect(player.autoplay).toBe(false);
});

it('still lets the user open the player when only first-frame extraction fails', async () => {
  vi.mocked(firstVideoFrame).mockRejectedValueOnce(new Error('unsupported thumbnail'));
  await act(async () => root.render(<Markdown text="![视频](clip.mp4)"/>));
  expect(container.textContent).toContain('首帧暂不可用');
  expect(container.querySelector('.chat-media-error')).toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('.chat-video-start')!.click());
  expect(container.querySelector('video')?.src).toContain('/full');
});

it('can reload a video that did not exist until its rendering finished', async () => {
  vi.mocked(call).mockRejectedValueOnce(new Error('媒体文件不存在，可能已移动或删除'));
  await act(async () => root.render(<Markdown text="![视频](renders/clip.mp4)"/>));
  expect(container.textContent).toContain('媒体文件不存在');
  await act(async () => container.querySelector<HTMLButtonElement>('.chat-media-error button')!.click());
  expect(vi.mocked(call).mock.calls.filter(([command]) => command === 'prepare_media')).toHaveLength(2);
  expect(container.querySelector('.chat-media-error')).toBeNull();
  expect(container.querySelector('.chat-video-poster')).not.toBeNull();
});

it('does not create a failed card for an old filename merely mentioned in a sentence', async () => {
  await act(async () => root.render(<Markdown text="旧版是 `樱花草MV-阿梓-旧版表情变化.mp4`，要删掉吗？"/>));
  expect(container.textContent).toContain('樱花草MV-阿梓-旧版表情变化.mp4');
  expect(container.querySelector('.chat-media-card')).toBeNull();
  expect(call).not.toHaveBeenCalled();
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

it('dismisses the file menu while Explorer is opening and keeps a later menu intact when the request finishes', async () => {
  await resourceImage('D:/Downloads/兔子.png');
  let finish!: () => void;
  vi.mocked(call).mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
  const image = container.querySelector('img')!;
  await rightClick(image);
  await act(async () => folderItem().click());
  expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(call).toHaveBeenLastCalledWith('reveal_media', { path:'D:/Downloads/兔子.png', projectId:null, sessionId:null });
  await rightClick(image);
  const nextMenu = document.querySelector('[role="menu"]');
  expect(folderItem().disabled).toBe(false);
  await act(async () => finish());
  expect(document.querySelector('[role="menu"]')).toBe(nextMenu);
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
  expect(document.querySelector('[role="menu"]')).toBeNull();
  expect(container.querySelector('[role="alert"]')?.textContent).toContain('文件不存在');
  expect(container.querySelector('img')).toBe(image);
  await act(async () => container.querySelector<HTMLButtonElement>('[title="关闭打开失败提示"]')!.click());
  expect(container.querySelector('[role="alert"]')).toBeNull();
});

it('does not offer a local folder for remote-only images', async () => {
  await resourceImage('https://example.com/bunny.png');
  await rightClick(container.querySelector('img')!);
  expect(folderItem()).toBeUndefined();
  expect(document.querySelector('[role="menu"]')?.textContent).toContain('复制链接');
  expect(call).not.toHaveBeenCalledWith('reveal_media',expect.anything());
});
