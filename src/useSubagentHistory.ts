import { useEffect, useState } from 'react';
import { call, desktop, subscribe } from './api';
import { isDisplayEvent, mergeEvent } from './events';
import type { Message, RpcEvent } from './types';

export interface SubagentHistory {
  messages:Message[]; model?:string; effort?:string; status?:string; turnId?:string; truncated?:boolean;
}
export function boundedSubagentMessages(messages:Message[]) {
  let bytes=0, start=messages.length;
  while(start>0 && messages.length-start<200) {
    const message=messages[start-1], size=message.text.length+JSON.stringify(message.data).length;
    if(bytes+size>2*1024*1024) break;
    bytes+=size; start--;
  }
  return messages.slice(start);
}
export function useSubagentHistory(sessionId:string, threadId:string|undefined, revision:number) {
  const [view,setView]=useState<SubagentHistory & {loading:boolean;error:string}>({messages:[],loading:false,error:''});
  useEffect(()=>{
    let disposed=false, off:(()=>void)|undefined, timer:ReturnType<typeof setTimeout>|undefined;
    let loading=true, replay:RpcEvent[]=[], queued:RpcEvent[]=[], replayBytes=0;
    setView({messages:[],loading:!!threadId&&desktop,error:''});
    if(!threadId||!desktop) return;
    function apply(old:typeof view, events:RpcEvent[]) {
      let next={...old,messages:boundedSubagentMessages(events.reduce((items,event)=>mergeEvent(items,event,sessionId),old.messages))};
      for(const event of events) {
        if(event.method==='turn/started') { next.status='inProgress';next.turnId=event.params.turn.id; }
        if(event.method==='turn/completed') { next.status=event.params.turn.status;next.turnId=event.params.turn.id; }
      }
      return next;
    }
    function flush() { timer=undefined; const events=queued;queued=[];if(!disposed&&events.length)setView(old=>apply(old,events)); }
    // Subscribe before the read, and replay updates received while it loads so a
    // late snapshot cannot replace newer streamed output or completion.
    void (async()=>{
      const cleanup=await subscribe(event=>{
        if(disposed || (event.params.threadId??event.params.thread?.id)!==threadId || !isDisplayEvent(event)) return;
        queued.push(event);
        if(loading) {
          const size=JSON.stringify(event).length;
          replay.push(event);replayBytes+=size;
          while(replay.length>2000 || replayBytes>2*1024*1024) replayBytes-=JSON.stringify(replay.shift()).length;
        }
        if(!timer)timer=setTimeout(flush,50);
      },()=>{},()=>{if(!disposed)setView(old=>({...old,error:'连接已断开，可重新加载子代理记录。'}));});
      if(disposed){cleanup();return;} off=cleanup;
      const result=await call<SubagentHistory>('get_subagent_messages',{sessionId,threadId});
      if(disposed)return;
      loading=false;
      if(timer)clearTimeout(timer);timer=undefined;queued=[];
      setView(apply({...result,messages:boundedSubagentMessages(result.messages),loading:false,error:''},replay));
      replay=[];
    })().catch(error=>{loading=false;replay=[];if(!disposed)setView(old=>({...old,loading:false,error:String(error)}));});
    return ()=>{disposed=true;off?.();if(timer)clearTimeout(timer);queued=[];replay=[];};
  },[sessionId,threadId,revision]);
  return view;
}
