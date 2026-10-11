import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { CornerDownRight, ListEnd, MoreHorizontal, Pencil, Plus, Trash2, X, Paperclip } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop } from './api';
import type { Attachment } from './ComposerMenus';
import { AttachmentImage } from './AttachmentImage';

export interface FollowupPayload { text:string; model:string|null; readOnly:boolean; permissionMode:string; attachments:Attachment[]; effort:string|null }
export interface Followup { id:string; sessionId:string; payload:FollowupPayload; status:string; error:string|null }
export function useFollowups(sessionId:string, report:(e:unknown)=>void) {
  const [rows,setRows]=useState<Followup[]>([]);
  const [steeringMode,setSteeringMode]=useState<string>();
  const error=useRef(report); error.current=report;
  const revision=useRef(0);
  const reload=useCallback(async()=>{
    const rev=++revision.current;
    if(!sessionId||!desktop){setRows([]);setSteeringMode(undefined);return;}
    try{const [data,caps]=await Promise.all([call<Followup[]>('list_followups',{sessionId}),call<{steeringMode:string}>('followup_capabilities',{sessionId})]);if(rev===revision.current){setRows(data);setSteeringMode(caps.steeringMode);}}catch(e){error.current(e);}
  },[sessionId]);
  useEffect(()=>{let disposed=false;let off:(()=>void)|undefined;void reload();if(desktop)void listen('followups-updated',()=>void reload()).then(fn=>{if(disposed)fn();else off=fn;});return()=>{disposed=true;revision.current++;off?.();};},[reload]);
  return {rows,reload,steeringMode};
}
export function FollowupQueue({rows,agent,turnId,steeringMode,change,steer,openSide,edit,editingId}:{rows:Followup[];agent:string;turnId?:string|null;steeringMode?:string;change:(id:string,action:string,text?:string)=>Promise<void>;steer:(id:string)=>Promise<void>;openSide:(row:Followup)=>void;edit:(row:Followup)=>Promise<void>;editingId?:string}) {
  const interrupt=steeringMode==='interrupt'||(steeringMode===undefined&&agent==='opencode');
  const [working,setWorking]=useState('');
  async function run(id:string,fn:()=>Promise<void>){if(working)return;setWorking(id);try{await fn();}catch{/* The caller displays the error and retains the message. */}finally{setWorking('');}}
  const visible=rows.filter(row=>row.id!==editingId);
  if(!visible.length)return null;
  return <section className="followup-queue" aria-label="排队消息"><div className="followup-list">{visible.map(row=>{
    const locked=!!working||['sending','steering','editing'].includes(row.status);
    const image=row.payload.attachments.find(attachment=>attachment.kind==='image');
    return <div className="followup-row" key={row.id} data-followup-id={row.id}>
      <ListEnd size={15} className="followup-icon"/>
      {image?<div className="followup-thumbnail"><AttachmentImage attachment={image}/>{row.payload.attachments.length>1?<span>{row.payload.attachments.length}</span>:null}</div>:row.payload.attachments.length?<span className="followup-attachment" title={`${row.payload.attachments.length} 个附件`}><Paperclip size={15}/><small>{row.payload.attachments.length}</small></span>:null}
        <div className="followup-content"><span title={row.payload.text}>{row.payload.text}</span>{row.error?<small role="alert">{row.error}</small>:row.status==='paused'?<small>已暂停</small>:null}</div>
        <div className="followup-actions">{['paused','failed'].includes(row.status)?<button disabled={locked} onClick={()=>void run(row.id,()=>change(row.id,'resume'))}>继续排队</button>:null}
          <button disabled={!turnId||locked} title={interrupt?'停止当前任务并发送这条消息':'补充当前任务，在 Agent 下一可接收的位置生效'} onClick={()=>void run(row.id,()=>steer(row.id))}><CornerDownRight size={15}/>{interrupt?'停止并引导':'引导'}</button>
          <button className="icon-button" aria-label="删除排队消息" disabled={locked} onClick={()=>void run(row.id,()=>change(row.id,'remove'))}><Trash2 size={15}/></button>
          <FollowupMenu locked={locked} editDisabled={!!editingId} edit={()=>void run(row.id,()=>edit(row))} openSide={()=>openSide(row)} pause={()=>void run(row.id,()=>change(row.id,'pause'))}/>
        </div>
    </div>;
  })}</div></section>;
}

function FollowupMenu({locked,editDisabled,edit,openSide,pause}:{locked:boolean;editDisabled:boolean;edit:()=>void;openSide:()=>void;pause:()=>void}) {
  const [open,setOpen]=useState(false), [position,setPosition]=useState({top:0,left:0});
  const trigger=useRef<HTMLButtonElement>(null), menu=useRef<HTMLDivElement>(null), id=useId();
  function close(restore=false){setOpen(false);if(restore)trigger.current?.focus();}
  useEffect(()=>{if(locked)setOpen(false);},[locked]);
  useEffect(()=>{
    if(!open)return;
    document.dispatchEvent(new CustomEvent('supercode:layer-open',{detail:id}));
    const outside=(event:Event)=>{if(!trigger.current?.contains(event.target as Node)&&!menu.current?.contains(event.target as Node))close();};
    const another=(event:Event)=>{if((event as CustomEvent).detail!==id)close();};
    const escape=(event:globalThis.KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();close(true);}};
    document.addEventListener('pointerdown',outside);document.addEventListener('focusin',outside);document.addEventListener('keydown',escape,true);document.addEventListener('supercode:layer-open',another);
    menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    return()=>{document.removeEventListener('pointerdown',outside);document.removeEventListener('focusin',outside);document.removeEventListener('keydown',escape,true);document.removeEventListener('supercode:layer-open',another);};
  },[open,id]);
  useLayoutEffect(()=>{
    if(!open)return;
    const update=()=>{const bounds=trigger.current?.getBoundingClientRect(),popup=menu.current;if(!bounds||!popup)return;const width=popup.offsetWidth||218,height=popup.offsetHeight||128;setPosition({left:Math.max(8,Math.min(bounds.right-width,window.innerWidth-width-8)),top:Math.max(8,bounds.top-height-6>=8?bounds.top-height-6:Math.min(bounds.bottom+6,window.innerHeight-height-8))});};
    update();window.addEventListener('resize',update);window.addEventListener('scroll',update,true);
    return()=>{window.removeEventListener('resize',update);window.removeEventListener('scroll',update,true);};
  },[open]);
  function navigate(event:KeyboardEvent){if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;const items=Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')??[]);if(!items.length)return;event.preventDefault();const current=items.indexOf(document.activeElement as HTMLButtonElement);items[event.key==='Home'?0:event.key==='End'?items.length-1:event.key==='ArrowDown'?(current+1)%items.length:(current+items.length-1)%items.length].focus();}
  return <><button ref={trigger} type="button" className="icon-button" aria-label="排队消息更多操作" aria-haspopup="menu" aria-expanded={open} disabled={locked} onClick={()=>setOpen(value=>!value)}><MoreHorizontal size={16}/></button>{open?createPortal(<div ref={menu} className="followup-menu" role="menu" aria-label="排队消息操作" style={position} onKeyDown={navigate}>
    <button type="button" role="menuitem" disabled={editDisabled} onClick={()=>{close();edit();}}><Pencil size={15}/>编辑消息</button>
    <button type="button" role="menuitem" onClick={()=>{close();openSide();}}><Plus size={15}/>在侧边聊天中打开</button>
    <button type="button" role="menuitem" onClick={()=>{close();pause();}}><X size={15}/>关闭排队</button>
  </div>,document.body):null}</>;
}
