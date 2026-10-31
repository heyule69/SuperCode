// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Event as TauriEvent } from '@tauri-apps/api/event';
import type { DragDropEvent } from '@tauri-apps/api/webview';
import { useComposerFileDrop } from './useComposerFileDrop';

const native = vi.hoisted(() => ({ subscribe:vi.fn() }));
vi.mock('./api', () => ({ desktop:true }));
vi.mock('@tauri-apps/api/webview', () => ({ getCurrentWebview:() => ({ onDragDropEvent:native.subscribe }) }));
let root:Root, node:HTMLDivElement;
let handlers:Set<(event:TauriEvent<DragDropEvent>)=>void>;
const add=vi.fn<(paths:string[])=>Promise<void>>(), report=vi.fn();
function Composer({id,enabled=true,inert=false,hidden=false,onAdd=add}:{id:string;enabled?:boolean;inert?:boolean;hidden?:boolean;onAdd?:(paths:string[])=>Promise<void>}) {
  const drop=useComposerFileDrop(onAdd,report,enabled);
  return <section inert={inert} hidden={hidden}><form ref={drop}><textarea id={id}/></form></section>;
}
beforeEach(()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  handlers=new Set(); add.mockReset().mockResolvedValue(); report.mockReset();
  native.subscribe.mockReset().mockImplementation(async handler=>{handlers.add(handler);return()=>{handlers.delete(handler);};});
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function drop(paths=['D:/预算表.xlsx']) {
  await act(async()=>{for(const handler of handlers) handler({payload:{type:'drop',paths,position:{x:300,y:180}}} as TauriEvent<DragDropEvent>);});
}
it('routes a physical-pixel drop only to the composer under the cursor at 150% scaling',async()=>{
  const main=vi.fn().mockResolvedValue(undefined),side=vi.fn().mockResolvedValue(undefined);
  await act(async()=>root.render(<><Composer id="main" onAdd={main}/><Composer id="side" onAdd={side}/></>));
  vi.stubGlobal('devicePixelRatio',1.5);
  const hit=vi.spyOn(document,'elementFromPoint').mockReturnValue(node.querySelector('#side'));
  await drop(['D:/报告.xls','D:/unknown.custom']);
  expect(hit).toHaveBeenCalledWith(200,120);
  expect(side).toHaveBeenCalledExactlyOnceWith(['D:/报告.xls','D:/unknown.custom']);
  expect(main).not.toHaveBeenCalled();
});
it.each(['disabled','inert','hidden','outside','overlay','empty'] as const)('ignores %s targets without touching attachments',async state=>{
  await act(async()=>root.render(<Composer id="input" enabled={state!=='disabled'} inert={state==='inert'} hidden={state==='hidden'}/>));
  const input=node.querySelector('#input');
  vi.spyOn(document,'elementFromPoint').mockReturnValue(state==='outside'?document.body:state==='overlay'?null:input);
  await drop(state==='empty'?[]:undefined);
  expect(add).not.toHaveBeenCalled();
});
it('uses the current draft callback after rerender and removes the native listener on unmount',async()=>{
  await act(async()=>root.render(<Composer id="input"/>));
  const next=vi.fn().mockResolvedValue(undefined);
  await act(async()=>root.render(<Composer id="input" onAdd={next}/>));
  vi.spyOn(document,'elementFromPoint').mockReturnValue(node.querySelector('#input'));
  await drop();
  expect(next).toHaveBeenCalledOnce();expect(add).not.toHaveBeenCalled();
  expect(native.subscribe).toHaveBeenCalledOnce();
  await act(async()=>root.render(null));expect(handlers.size).toBe(0);
});
it('reports attachment errors and releases a listener that finishes subscribing after unmount',async()=>{
  add.mockRejectedValue(new Error('附件不存在'));
  await act(async()=>root.render(<Composer id="input"/>));
  vi.spyOn(document,'elementFromPoint').mockReturnValue(node.querySelector('#input'));
  await drop();expect(report).toHaveBeenCalledWith(expect.objectContaining({message:'附件不存在'}));
  await act(async()=>root.render(null));
  let finish!:(stop:()=>void)=>void;
  native.subscribe.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  await act(async()=>root.render(<Composer id="input"/>));
  await act(async()=>root.render(null));
  const stop=vi.fn();await act(async()=>finish(stop));expect(stop).toHaveBeenCalledOnce();
});
