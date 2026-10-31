// @vitest-environment happy-dom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {TurnArtifacts} from './TurnArtifacts';
import {MediaContext} from './Media';
import {call} from './api';
import type {ArtifactFile} from './artifacts';
import {save} from '@tauri-apps/plugin-dialog';

vi.mock('./api',()=>({desktop:true,call:vi.fn()}));
vi.mock('@tauri-apps/plugin-dialog',()=>({save:vi.fn()}));
let root:Root,node:HTMLDivElement;
const file:ArtifactFile={path:'D:/中文目录 with spaces/公司信息.xlsx',name:'公司信息.xlsx',size:544032};
const open=vi.fn();
beforeEach(()=>{(globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;vi.mocked(call).mockReset().mockResolvedValue([]);vi.mocked(save).mockReset();open.mockReset();node=document.createElement('div');document.body.append(node);root=createRoot(node);});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
async function render(paths:string[],sessionId='side',error?:string) {
  await act(async()=>root.render(<MediaContext.Provider value={{projectId:'',sessionId,openFile:open}}><TurnArtifacts pathsKey={JSON.stringify(paths)} collectionError={error}/></MediaContext.Provider>));
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
