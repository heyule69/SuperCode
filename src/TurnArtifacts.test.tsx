// @vitest-environment happy-dom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {TurnArtifacts} from './TurnArtifacts';
import {MediaContext} from './Media';
import {call} from './api';
import type {ArtifactFile} from './artifacts';
import {save} from '@tauri-apps/plugin-dialog';
import {artifactLink,type ArtifactLink} from './artifactLinks';

vi.mock('./api',()=>({desktop:true,call:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({save:vi.fn()}));
let root:Root,node:HTMLDivElement;
const file:ArtifactFile={path:'D:/中文目录 with spaces/公司信息.xlsx',name:'公司信息.xlsx',size:544032};
const open=vi.fn();
const writeText=vi.fn(async(_text:string)=>{});
beforeEach(()=>{(globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;vi.mocked(call).mockReset().mockResolvedValue([]);vi.mocked(save).mockReset();open.mockReset();writeText.mockReset().mockResolvedValue(undefined);Object.defineProperty(navigator,'clipboard',{value:{writeText},configurable:true});node=document.createElement('div');document.body.append(node);root=createRoot(node);});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
async function render(paths:string[],sessionId='side',error?:string,options:{links?:ArtifactLink[];known?:ArtifactFile[]}={}) {
  await act(async()=>root.render(<MediaContext.Provider value={{projectId:'',sessionId,openFile:open}}><TurnArtifacts pathsKey={JSON.stringify(paths)} linksKey={JSON.stringify(options.links??[])} knownKey={JSON.stringify(options.known??[])} collectionError={error}/></MediaContext.Provider>));
}
it('displays only verified existing files and supports opening, locating and saving a real copy',async()=>{
  vi.mocked(call).mockImplementation(async command=>command==='resolve_artifact_references'?[file]:undefined);
  await render([file.path,'D:/missing.xlsx']);
  expect(call).toHaveBeenCalledWith('resolve_artifact_references',{paths:[file.path,'D:/missing.xlsx'],projectId:'',sessionId:'side'});
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(1);expect(node.textContent).toContain('XLSX · 531.3 KB');
  await act(async()=>node.querySelector<HTMLButtonElement>('.artifact-open')!.click());expect(open).toHaveBeenCalledWith(file.path);
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label^="定位"]')!.click());
  expect(call).toHaveBeenCalledWith('open_project_path',{path:file.path,action:'reveal',projectId:'',sessionId:'side'});
  vi.mocked(save).mockResolvedValue('D:/另存副本.xlsx');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label^="另存为"]')!.click());
  expect(call).toHaveBeenCalledWith('save_artifact_copy',{path:file.path,destination:'D:/另存副本.xlsx',projectId:'',sessionId:'side'});
  expect(node.querySelector('[title="已另存"]')).not.toBeNull();
});
it('renders no card or empty section for normal chat, missing files or cancelled save dialogs',async()=>{
  await render([]);expect(node.innerHTML).toBe('');expect(call).not.toHaveBeenCalled();
  await render(['D:/missing.xlsx']);expect(node.innerHTML).toBe('');
  vi.mocked(call).mockImplementation(async command=>command==='resolve_artifact_references'?[file]:undefined);
  await render([file.path]);vi.mocked(save).mockResolvedValue(null);
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label^="另存为"]')!.click());
  expect(call).not.toHaveBeenCalledWith('save_artifact_copy',expect.anything());
});
it('does not reload metadata on an unchanged render and ignores late results from another conversation',async()=>{
  let finish!:(files:ArtifactFile[])=>void;
  vi.mocked(call).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
  await render([file.path],'first');
  await render([file.path],'first');expect(call).toHaveBeenCalledOnce();
  await render(['D:/different.xlsx'],'second');
  await act(async()=>finish([file]));expect(node.querySelector('.artifact-file')).toBeNull();
});
it('reports collection errors without producing a fake file card',async()=>{
  await render([],'side','无法检查交付目录');
  expect(node.querySelector('.artifact-file')).toBeNull();expect(node.textContent).toBe('无法检查交付目录');
});

const pictures:ArtifactFile[]=Array.from({length:24},(_,index)=>({path:`D:/outputs/${index+1}-wizard.png`,name:`${index+1}-wizard.png`,size:300*1024}));
const documentFile:ArtifactFile={path:'D:/outputs/大肥鱼安装教程.md',name:'大肥鱼安装教程.md',size:18000};
it('collects many files into one panel with documents first and a collapsed image group',async()=>{
  const files=[...pictures,documentFile];
  vi.mocked(call).mockResolvedValue(files);
  await render(files.map(file=>file.path));
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(1);
  expect(node.querySelector('.artifact-open')?.textContent).toContain(documentFile.name);
  expect(node.textContent).toContain('25 个文件');
  expect(node.textContent).toContain('24 张');
  expect(node.querySelector<HTMLButtonElement>('[aria-label="展开图片"]')?.getAttribute('aria-expanded')).toBe('false');
  expect(node.querySelector('[data-resource-path="D:/outputs/1-wizard.png"]')).toBeNull();
  expect(call).toHaveBeenCalledOnce();
  expect(call).not.toHaveBeenCalledWith('prepare_media',expect.anything());
});

it('paginates image filenames on demand and releases their rows when collapsed',async()=>{
  vi.mocked(call).mockResolvedValue(pictures);
  await render(pictures.map(file=>file.path));
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开图片"]')!.click());
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(8);
  expect(node.querySelector('[data-resource-path="D:/outputs/1-wizard.png"]')).not.toBeNull();
  expect(node.querySelector('[data-resource-path="D:/outputs/9-wizard.png"]')).toBeNull();
  expect(node.textContent).toContain('1–8 / 24');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="下一页图片"]')!.click());
  expect(node.querySelector('[data-resource-path="D:/outputs/9-wizard.png"]')).not.toBeNull();
  expect(node.querySelector('[data-resource-path="D:/outputs/1-wizard.png"]')).toBeNull();
  await act(async()=>node.querySelector<HTMLButtonElement>('.artifact-open')!.click());
  expect(open).toHaveBeenCalledWith('D:/outputs/9-wizard.png');
  await render(pictures.map(file=>file.path));
  expect(node.textContent).toContain('9–16 / 24');
  expect(call).toHaveBeenCalledOnce();
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="收起图片"]')!.click());
  expect(node.querySelector('.artifact-file')).toBeNull();
  expect(node.querySelector('[aria-label="图片分页"]')).toBeNull();
  expect(call).not.toHaveBeenCalledWith('prepare_media',expect.anything());
});

it('keeps small deliveries visible and limits the default preview of a large document group',async()=>{
  const files=Array.from({length:12},(_,index)=>({...file,path:`D:/outputs/报告-${index}.xlsx`,name:`报告-${index}.xlsx`}));
  vi.mocked(call).mockResolvedValueOnce(files.slice(0,3));
  await render(files.slice(0,3).map(file=>file.path));
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(3);
  expect(node.querySelector('.artifact-collection')).toBeNull();
  vi.mocked(call).mockResolvedValueOnce(files);
  await render(files.map(file=>file.path));
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(3);
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开文件"]')!.click());
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(8);
  expect(node.textContent).toContain('1–8 / 12');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="下一页文件"]')!.click());
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(4);
  expect(node.querySelector<HTMLButtonElement>('[aria-label="下一页文件"]')!.disabled).toBe(true);
});

it('groups video and audio separately and retains file actions inside a paginated group',async()=>{
  const video={path:'D:/outputs/demo.mp4',name:'demo.mp4',size:1024000};
  const audio={path:'D:/outputs/voice.mp3',name:'voice.mp3',size:512000};
  const files=[...pictures,documentFile,video,audio];
  vi.mocked(call).mockImplementation(async command=>command==='resolve_artifact_references'?files:undefined);
  await render(files.map(file=>file.path));
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(1);
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开视频"]')!.click());
  expect(node.querySelector(`[data-resource-path="${video.path}"]`)).not.toBeNull();
  expect(node.querySelector(`[data-resource-path="${audio.path}"]`)).toBeNull();
  await act(async()=>node.querySelector<HTMLButtonElement>(`[aria-label="定位 ${video.name}"]`)!.click());
  expect(call).toHaveBeenCalledWith('open_project_path',{path:video.path,action:'reveal',projectId:'',sessionId:'side'});
  vi.mocked(save).mockResolvedValue('D:/copy/demo.mp4');
  await act(async()=>node.querySelector<HTMLButtonElement>(`[aria-label="另存为 ${video.name}"]`)!.click());
  expect(call).toHaveBeenCalledWith('save_artifact_copy',{path:video.path,destination:'D:/copy/demo.mp4',projectId:'',sessionId:'side'});
});

it('starts the new conversation with collapsed groups rather than reusing the previous image page',async()=>{
  vi.mocked(call).mockResolvedValue(pictures);
  await render(pictures.map(file=>file.path),'first');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开图片"]')!.click());
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="下一页图片"]')!.click());
  await render(pictures.map(file=>file.path),'second');
  expect(node.querySelector('.artifact-file')).toBeNull();
  expect(node.querySelector('[aria-label="展开图片"]')?.getAttribute('aria-expanded')).toBe('false');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开图片"]')!.click());
  expect(node.textContent).toContain('1–8 / 24');
});

it('renders website and local-preview cards immediately without requesting file metadata or external previews',async()=>{
  const links=[artifactLink('https://demo.example.com/app?token=private','项目主页')!,artifactLink('http://localhost:3000/','本地演示')!];
  await render([],'side',undefined,{links});
  expect(call).not.toHaveBeenCalled();
  expect(node.querySelectorAll('.artifact-link')).toHaveLength(2);
  expect(node.textContent).toContain('项目主页');expect(node.textContent).toContain('本地预览');
  expect(node.textContent).not.toContain('private');expect(node.textContent).not.toContain('0 B');
  expect(node.querySelector('iframe,img,video')).toBeNull();
  expect(writeText).not.toHaveBeenCalled();
  await act(async()=>node.querySelector<HTMLAnchorElement>('.artifact-link a')!.click());
  expect(call).toHaveBeenCalledWith('open_external_link',{url:links[0].url});
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="复制链接 项目主页"]')!.click());
  expect(writeText).toHaveBeenCalledWith(links[0].url);expect(node.textContent).toContain('已复制');
});

it('uses a download-link card rather than pretending a remote file is a verified local download',async()=>{
  const link=artifactLink('https://cdn.example.com/package.zip','下载源码')!;
  await render([],'side',undefined,{links:[link]});
  expect(node.textContent).toContain('下载链接');
  expect(node.querySelector('[aria-label^="另存为"]')).toBeNull();
  await act(async()=>node.querySelector<HTMLAnchorElement>('.artifact-link a')!.click());
  expect(call).toHaveBeenCalledWith('open_external_link',{url:link.url});
  expect(save).not.toHaveBeenCalled();
});

it('keeps web cards available after a file check fails and lets the user retry metadata',async()=>{
  const link=artifactLink('https://demo.example.com','项目预览')!;
  vi.mocked(call).mockRejectedValueOnce(new Error('无法访问产物')).mockResolvedValueOnce([file]);
  await render([file.path],'side',undefined,{links:[link]});
  expect(node.querySelector('.artifact-link')).not.toBeNull();
  expect(node.textContent).toContain('无法访问产物');
  await act(async()=>[...node.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==='重新检查产物')!.click());
  expect(node.textContent).toContain(file.name);expect(node.textContent).not.toContain('无法访问产物');
});

it('shows a missing historical artifact without inventing a card for an unverified text reference',async()=>{
  const known={path:'D:/outputs/gone.xlsx',name:'gone.xlsx',size:1024};
  vi.mocked(call).mockResolvedValueOnce([]).mockResolvedValueOnce([known]);
  await render([known.path,'D:/never-generated.xlsx'],'side',undefined,{known:[known]});
  expect(node.querySelectorAll('.artifact-file')).toHaveLength(1);
  expect(node.textContent).toContain('文件已移动或删除');
  expect(node.querySelector<HTMLButtonElement>('.artifact-open')!.disabled).toBe(true);
  expect(node.querySelector('[aria-label^="另存为"]')).toBeNull();
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="重新检查 gone.xlsx"]')!.click());
  expect(node.textContent).not.toContain('文件已移动或删除');
  expect(node.querySelector<HTMLButtonElement>('.artifact-open')!.disabled).toBe(false);
});

it('opens directories and HTML pages appropriately while program and unknown files are only revealed',async()=>{
  const folder:ArtifactFile={path:'D:/outputs/project',name:'project',size:0,kind:'directory'};
  const html:ArtifactFile={path:'D:/outputs/index.html',name:'index.html',size:1024};
  const program:ArtifactFile={path:'D:/outputs/setup.exe',name:'setup.exe',size:3*1024**3};
  const unknown:ArtifactFile={path:'D:/outputs/empty.custom',name:'empty.custom',size:0};
  const files=[folder,html,program,unknown];
  vi.mocked(call).mockImplementation(async command=>command==='resolve_artifact_references'?files:undefined);
  await render(files.map(file=>file.path));
  const button=(path:string)=>node.querySelector<HTMLButtonElement>(`[data-resource-path="${path}"] .artifact-open`)!;
  expect(node.querySelector(`[data-resource-path="${folder.path}"]`)?.textContent).not.toContain('0 B');
  expect(node.querySelector(`[data-resource-path="${folder.path}"] [aria-label^="另存为"]`)).toBeNull();
  for(const entry of [folder,html]){await act(async()=>button(entry.path).click());expect(call).toHaveBeenCalledWith('open_project_path',{path:entry.path,action:'open',projectId:'',sessionId:'side'});}
  for(const entry of [program,unknown]){await act(async()=>button(entry.path).click());expect(call).toHaveBeenCalledWith('open_project_path',{path:entry.path,action:'reveal',projectId:'',sessionId:'side'});}
  expect(node.textContent).toContain('3.0 GB');expect(node.textContent).toContain('空文件');
  expect(open).not.toHaveBeenCalled();
});

it('keeps duplicate filenames distinguishable and bounds mixed-link collections',async()=>{
  const files=[{...file,path:'D:/outputs/current/报告.xlsx',name:'报告.xlsx'},{...file,path:'D:/outputs/archive/报告.xlsx',name:'报告.xlsx'}];
  vi.mocked(call).mockResolvedValue(files);
  const links=Array.from({length:20},(_,index)=>artifactLink(`https://example.com/${index}`,`网页 ${index}`)!);
  await render(files.map(file=>file.path),'side',undefined,{links});
  expect(node.textContent).toContain('22 项');
  expect(node.querySelectorAll('.artifact-link')).toHaveLength(3);
  expect(node.textContent).toContain('current/报告.xlsx');expect(node.textContent).toContain('archive/报告.xlsx');
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="展开链接"]')!.click());
  expect(node.querySelectorAll('.artifact-link')).toHaveLength(8);
  expect(node.textContent).toContain('1–8 / 20');
});

it('lets link opens be retried and does not leak a late action failure into another chat',async()=>{
  const link=artifactLink('https://demo.example.com','网页预览')!;
  await render([],'first',undefined,{links:[link]});
  vi.mocked(call).mockRejectedValueOnce(new Error('浏览器暂不可用'));
  await act(async()=>node.querySelector<HTMLAnchorElement>('.artifact-link a')!.click());
  expect(node.textContent).toContain('浏览器暂不可用');
  let reject!:(error:Error)=>void;
  vi.mocked(call).mockImplementationOnce(()=>new Promise((_resolve,fail)=>{reject=fail;}));
  await act(async()=>[...node.querySelectorAll<HTMLButtonElement>('button')].find(button=>button.textContent==='重试操作')!.click());
  expect(node.textContent).not.toContain('浏览器暂不可用');
  await render([],'second',undefined,{links:[link]});
  await act(async()=>reject(new Error('旧操作失败')));
  expect(node.textContent).not.toContain('旧操作失败');
});

it('ignores invalid link data rather than letting it create an executable card or crash the reply',async()=>{
  await act(async()=>root.render(<TurnArtifacts pathsKey="[]" linksKey="invalid JSON"/>));
  expect(node.innerHTML).toBe('');
  await act(async()=>root.render(<TurnArtifacts pathsKey="[]" linksKey={JSON.stringify([{url:'javascript:alert(1)'},{url:'https://user:secret@example.com'}])}/>));
  expect(node.innerHTML).toBe('');
});
