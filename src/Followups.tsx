import { useCallback, useEffect, useRef, useState } from 'react';
import { CornerDownRight, ListEnd, MoreHorizontal, Pencil, Plus, Trash2, X } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop } from './api';
import type { Attachment } from './ComposerMenus';

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
export function FollowupQueue({rows,agent,turnId,steeringMode,change,steer,openSide}:{rows:Followup[];agent:string;turnId?:string|null;steeringMode?:string;change:(id:string,action:string,text?:string)=>Promise<void>;steer:(id:string)=>Promise<void>;openSide:(row:Followup)=>void}) {
  const interrupt=steeringMode==='interrupt'||(steeringMode===undefined&&agent==='opencode');
  const [editing,setEditing]=useState('');const [text,setText]=useState('');const [working,setWorking]=useState('');
  async function run(id:string,fn:()=>Promise<void>){if(working)return;setWorking(id);try{await fn();}catch{/* The caller displays the error and retains the message. */}finally{setWorking('');}}
  if(!rows.length)return null;
  return <section className="followup-queue" aria-label="排队消息">{rows.map(row=>{
    const locked=!!working||['sending','steering'].includes(row.status);
    return <div className="followup-row" key={row.id}>
      <ListEnd size={16} className="followup-icon"/>
      {editing===row.id?<form className="followup-editor" onSubmit={e=>{e.preventDefault();void run(row.id,async()=>{await change(row.id,'edit',text);setEditing('');});}}><textarea autoFocus aria-label="编辑排队消息" value={text} onChange={e=>setText(e.target.value)} rows={2}/><div><button type="button" onClick={()=>setEditing('')}>取消</button><button type="submit" disabled={locked||!text.trim()}>保存</button></div></form>:<>
        <div className="followup-content"><span title={row.payload.text}>{row.payload.text}</span>{row.payload.attachments.length?<small>{row.payload.attachments.length} 个附件</small>:null}{row.error?<small role="alert">{row.error}</small>:row.status==='paused'?<small>排队已暂停</small>:row.status==='steering'?<small>等待 Agent 确认…</small>:locked?<small>正在发送…</small>:null}</div>
        <div className="followup-actions">{['paused','failed'].includes(row.status)?<button disabled={locked} onClick={()=>void run(row.id,()=>change(row.id,'resume'))}>继续排队</button>:null}
          <button disabled={!turnId||locked} title={interrupt?'停止当前任务并发送这条消息':'补充当前任务，在 Agent 下一可接收的位置生效'} onClick={()=>void run(row.id,()=>steer(row.id))}><CornerDownRight size={15}/>{interrupt?'停止并引导':'引导'}</button>
          <button className="icon-button" aria-label="删除排队消息" disabled={locked} onClick={()=>void run(row.id,()=>change(row.id,'remove'))}><Trash2 size={15}/></button>
          <details className="followup-more"><summary aria-label="排队消息更多操作"><MoreHorizontal size={16}/></summary><div role="menu"><button role="menuitem" disabled={locked} onClick={e=>{e.currentTarget.closest('details')?.removeAttribute('open');setEditing(row.id);setText(row.payload.text);}}><Pencil size={15}/>编辑消息</button><button role="menuitem" disabled={locked} onClick={e=>{e.currentTarget.closest('details')?.removeAttribute('open');openSide(row);}}><Plus size={15}/>在侧边聊天中打开</button><button role="menuitem" disabled={locked} onClick={e=>{e.currentTarget.closest('details')?.removeAttribute('open');void run(row.id,()=>change(row.id,'pause'));}}><X size={15}/>关闭排队</button></div></details>
        </div>
      </>}
    </div>;
  })}</section>;
}
