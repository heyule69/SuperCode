import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowLeft, ArrowRight, Globe, LoaderCircle, Maximize2, Minimize2, RefreshCw, Square, X } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop } from './api';
import './browser-panel.css';

export type BrowserState = { sessionId:string; url:string; title:string; status:'idle'|'working'|'stopped'|'error'; action:string };
type Location = { sessionId:string; url:string; title?:string; loading?:boolean };
export function browserAddress(value:string) {
  const text=value.trim();const address=/^[a-z][a-z\d+.-]*:/i.test(text)?text:`https://${text}`;
  const url=new URL(address);
  if (!['http:','https:'].includes(url.protocol) || !url.hostname || url.username || url.password) throw new Error('请输入 http 或 https 网页地址');
  return url.href;
}
export default function BrowserPanel({ sessionId, visible, close, resizeHandle, expanded, expand, embedded=false }:{sessionId:string; visible:boolean; close:()=>void; resizeHandle?:ReactNode; expanded:boolean; expand:()=>void; embedded?:boolean}) {
  const area=useRef<HTMLDivElement>(null);
  const [state,setState]=useState<BrowserState>({sessionId,url:'about:blank',title:'新标签页',status:'idle',action:''});
  const [address,setAddress]=useState('');const [error,setError]=useState('');const [ready,setReady]=useState(false);const [loading,setLoading]=useState(false);
  const [navigating,setNavigating]=useState(false);const addressFocused=useRef(false);const visibility=useRef(visible);visibility.current=visible;
  const requestLayout=useRef<()=>void>(()=>{});
  const mounted=useRef('');const live=useRef(false);
  useEffect(()=>{
    const mountId=crypto.randomUUID();let disposed=false;let opened=false;const offs:Array<()=>void>=[];let observer:ResizeObserver|undefined;let frame=0;
    mounted.current=mountId;live.current=true;
    const layout=()=>{
      if(!opened || disposed)return;
      cancelAnimationFrame(frame);frame=requestAnimationFrame(()=>{
        if(disposed || !area.current)return;const r=area.current.getBoundingClientRect();
        void call('browser_layout',{sessionId,mountId,x:r.x,y:r.y,width:r.width,height:r.height,visible:visibility.current && document.visibilityState!=='hidden'}).catch(e=>{if(!disposed)setError(String(e));});
      });
    };
    requestLayout.current=layout;
    void (async()=>{
      try {
        if(!desktop)throw new Error('内置浏览器在 SuperCode 桌面端可用');
        const a=await listen<BrowserState>('browser-state',e=>{if(!disposed && e.payload.sessionId===sessionId)setState(old=>({...old,...e.payload,url:e.payload.url||old.url,title:e.payload.title||old.title}));});
        if(disposed){a();return;}offs.push(a);
        const b=await listen<Location>('browser-location',e=>{if(disposed || e.payload.sessionId!==sessionId)return;setState(old=>({...old,url:e.payload.url,title:e.payload.title||old.title}));if(!addressFocused.current)setAddress(e.payload.url==='about:blank'?'':e.payload.url);setLoading(!!e.payload.loading);});
        if(disposed){b();return;}offs.push(b);
        const result=await call<BrowserState>('browser_open',{sessionId,mountId});
        opened=true;
        if(disposed){void call('browser_close',{sessionId,mountId}).catch(()=>{});return;}
        setState(old=>({...old,...result,url:result.url||old.url,title:result.title||old.title}));setReady(true);
        setAddress(result.url==='about:blank'?'':result.url);
        observer=new ResizeObserver(layout);if(area.current){observer.observe(area.current);const surface=area.current.closest('.conversation-surface');if(surface)observer.observe(surface);}layout();
        window.addEventListener('resize',layout);document.addEventListener('visibilitychange',layout);
      }catch(e){if(!disposed)setError(String(e));}
    })();
    return ()=>{disposed=true;live.current=false;cancelAnimationFrame(frame);offs.forEach(off=>off());observer?.disconnect();window.removeEventListener('resize',layout);document.removeEventListener('visibilitychange',layout);if(opened)void call('browser_close',{sessionId,mountId}).catch(()=>{});};
  },[sessionId]);
  useEffect(()=>requestLayout.current(),[visible,expanded,ready]);
  useEffect(()=>{
    const stop=(event:KeyboardEvent)=>{if(visibility.current && event.key==='Escape' && (state.status==='working'||state.status==='error')){event.preventDefault();void call('browser_takeover',{sessionId}).catch(e=>setError(String(e)));}};
    window.addEventListener('keydown',stop,true);return ()=>window.removeEventListener('keydown',stop,true);
  },[sessionId,state.status]);
  async function manual(action:string,url?:string){setError('');setNavigating(true);try{await call('browser_manual',{sessionId,action,address:url??null});}catch(e){setError(String(e));}finally{setNavigating(false);}}
  async function dismiss(){const mountId=mounted.current;try{await call('browser_close',{sessionId,mountId});if(live.current && mounted.current===mountId)close();}catch(e){if(live.current && mounted.current===mountId)setError(String(e));}}
  return <aside className={`browser-panel${expanded?' browser-expanded':''}`} hidden={!visible} aria-label="内置浏览器">
    {resizeHandle}
    {!embedded ? <div className="browser-panel-heading"><div className="browser-page-title"><Globe size={14}/><span title={state.title}>{state.title==='新标签页'?'浏览器':state.title||'浏览器'}</span></div><button className="icon-button" onClick={expand} title={expanded?'返回分屏':'全屏查看网页'} aria-label={expanded?'返回分屏':'全屏查看网页'}>{expanded?<Minimize2 size={15}/>:<Maximize2 size={15}/>}</button><button className="icon-button" onClick={()=>void dismiss()} aria-label="关闭浏览器" title="关闭浏览器，任务继续"><X size={16}/></button></div> : null}
    <form className="browser-toolbar" onSubmit={e=>{e.preventDefault();try{void manual('navigate',browserAddress(address));}catch(e){setError(String(e));}}}>
      <button type="button" className="icon-button" aria-label="网页后退" disabled={!ready||navigating} onClick={()=>void manual('back')}><ArrowLeft size={15}/></button>
      <button type="button" className="icon-button" aria-label="网页前进" disabled={!ready||navigating} onClick={()=>void manual('forward')}><ArrowRight size={15}/></button>
      <button type="button" className="icon-button" aria-label="刷新网页" disabled={!ready||navigating} onClick={()=>void manual('reload')}>{loading?<LoaderCircle size={15} className="spin"/>:<RefreshCw size={15}/>}</button>
      <input aria-label="网页地址" value={address} onFocus={()=>{addressFocused.current=true;}} onBlur={()=>{addressFocused.current=false;}} onChange={e=>setAddress(e.target.value)} placeholder="输入网址" spellCheck={false} disabled={!ready}/>
    </form>
    <div className="browser-status" role="status"><span className={`browser-status-dot ${state.status}`}/><span title={state.action}>{state.status==='working'?state.action:state.status==='stopped'?'你正在控制浏览器':state.status==='error'?state.action||'操作未完成':state.title||'浏览器已就绪'}</span>{state.status==='working'||state.status==='error'?<><kbd>Esc 停止</kbd><button className="browser-takeover" onClick={()=>void call('browser_takeover',{sessionId}).catch(e=>setError(String(e)))}><Square size={10} fill="currentColor"/>接管</button></>:null}</div>
    {error?<div className="browser-error" role="alert"><span>{error}</span><button className="icon-button" aria-label="关闭浏览器错误" onClick={()=>setError('')}><X size={13}/></button></div>:null}
    <div className="browser-viewport" ref={area}>{!ready?<div className="browser-empty"><Globe size={30}/><span>{error?'浏览器暂不可用':'正在打开浏览器…'}</span></div>:null}</div>
  </aside>;
}
