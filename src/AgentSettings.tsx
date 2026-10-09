import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Download, FolderOpen, RefreshCw, X } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop } from './api';
import { AgentIcon } from './AgentIcon';
import { releaseState, type AgentRelease } from './agentVersions';
import type { Agent } from './types';
import './agentSettings.css';

type Status = Agent & { phase?:string; message?:string; version?:string; source?:string; received?:number; total?:number };
const working = (s:Status) => ['downloading','installing','testing'].includes(s.phase??'');
export default function AgentSettings({agents,busy,loadMcp,setLoadMcp,saveMcp}:{agents:Agent[];busy:boolean;loadMcp:boolean;setLoadMcp:(value:boolean)=>void;saveMcp:()=>Promise<void>}) {
  const [items,setItems]=useState<Status[]>(agents);
  const [releases,setReleases]=useState<Record<string,AgentRelease>>({});
  const [loading,setLoading]=useState(desktop),[checking,setChecking]=useState(desktop);
  const [acting,setActing]=useState(false),[advanced,setAdvanced]=useState(false),[error,setError]=useState('');
  const mounted=useRef(false),revision=useRef(0);
  async function checkVersions(force=false) {
    const rev=++revision.current;
    setChecking(true);
    try {
      const rows=await call<AgentRelease[]>('check_agent_updates',{force});
      if(mounted.current&&revision.current===rev)setReleases(Object.fromEntries(rows.map(row=>[row.id,row])));
    } catch(e) {
      if(mounted.current&&revision.current===rev)setReleases(previous=>Object.fromEntries(agents.map(a=>[a.id,{...previous[a.id],id:a.id,latestVersion:previous[a.id]?.latestVersion??null,checkedAt:previous[a.id]?.checkedAt??null,error:String(e)}])));
    } finally {if(mounted.current&&revision.current===rev)setChecking(false);}
  }
  useEffect(()=>{
    mounted.current=true;
    if(!desktop)return()=>{mounted.current=false;};
    let disposed=false;let stop:(()=>void)|undefined;
    // Network checks do not delay local detection or overwrite installation events.
    void checkVersions();
    void(async()=>{
      try {
        const off=await listen<Status>('agent-install-progress',e=>{if(!disposed)setItems(rows=>rows.map(s=>s.id===e.payload.id?{...s,...e.payload}:s));});
        if(disposed){off();return;}stop=off;
        const rows=await call<Status[]>('list_agents');if(!disposed)setItems(rows);
      }catch(e){if(!disposed)setError(String(e));}
      finally{if(!disposed)setLoading(false);}
    })();
    return()=>{disposed=true;mounted.current=false;revision.current++;stop?.();};
  },[]);
  async function action(command:string,args:Record<string,unknown>={}) {
    setActing(true);setError('');
    try{const result=await call<Status[]>(command,args);if(mounted.current&&Array.isArray(result))setItems(result);}
    catch(e){if(mounted.current){setError(String(e));if(command==='install_agent'||command==='update_agent'){const rows=await call<Status[]>('list_agents').catch(()=>null);if(mounted.current&&rows)setItems(rows);}}}
    finally{if(mounted.current)setActing(false);}
  }
  async function browse(id:string) {
    const {open}=await import('@tauri-apps/plugin-dialog');
    const path=await open({title:'选择 Agent 程序',multiple:false,filters:[{name:'Agent',extensions:['exe','js','mjs','cmd']}]});
    if(path){await action('configure_agent',{id,path});await action('list_agents');}
  }
  const locked=busy||acting||loading||items.some(working);
  return <div className="client-settings-page agent-settings-page">
    <div className="settings-page-heading"><h2>Agent</h2><div className="agent-heading-actions"><button className="quiet-button" disabled={!desktop||checking} onClick={()=>void checkVersions(true)}><RefreshCw size={14} className={checking?'agent-checking':''}/>{checking?'检查中…':'检查更新'}</button><button className="quiet-button" disabled={!desktop||locked} onClick={()=>void action('list_agents')}>重新检测</button></div></div>
    <p className="agent-settings-intro">自动使用本机已安装的 Agent 和已有配置。可一键安装或更新，完成后自动测试连接。</p>
    {error?<p className="cc-error" role="alert">{error}</p>:null}
    <div className="agent-install-list" aria-busy={loading}>{items.map(s=>{
      const release=releases[s.id],state=releaseState(s.version,release?.latestVersion),available=!!release?.latestVersion&&!release.error;
      const healthy=s.installed&&s.phase!=='failed';
      return <section className="agent-install-row" key={s.id} aria-label={s.name}>
        <div className="agent-install-main"><span className="agent-install-logo"><AgentIcon agent={s.id}/></span><div className="agent-install-info"><h3>{s.name}</h3><p>{loading?'正在检测…':!desktop?'桌面端可检测和安装':working(s)?s.message:s.version?`${s.version} · ${s.source==='managed'?'SuperCode 安装':s.source==='configured'?'手动指定':'本机安装'}`:s.installed?'已检测到本机安装':'尚未安装'}</p>
          {desktop?<div className="agent-release-info">{checking?<span>正在检查最新版本…</span>:release?.latestVersion?<><span>{release.error?'上次检测':'最新版本'} <strong>{release.latestVersion}</strong></span>{!release.error&&healthy?<span className={state==='update'?'agent-update-badge':'agent-version-label'}>{state==='update'?'可更新':state==='current'?'已是最新':state==='newer'?'本机版本更新':''}</span>:null}</>:null}{!checking&&release?.error?<span className="agent-release-error" title={release.error}>暂时无法检查更新<button disabled={locked} onClick={()=>void checkVersions(true)}>重试</button></span>:null}</div>:null}
        </div><div className="agent-install-actions">{working(s)?<button className="quiet-button" onClick={()=>void call('cancel_agent_install',{id:s.id}).catch(e=>setError(String(e)))}><X size={14}/>取消</button>:<>
          {healthy?<span className="agent-install-ready"><Check size={14}/>可使用</span>:null}
          {healthy?<button className="quiet-button" disabled={locked||!desktop} onClick={()=>void action('test_agent',{id:s.id})}>测试连接</button>:null}
          {!healthy?<button className="primary-button" disabled={locked||!desktop||checking||!available} onClick={()=>void action('install_agent',{id:s.id,repair:s.phase==='failed',version:release?.latestVersion})}><Download size={14}/>{s.installed?'修复安装':'安装'}</button>:available&&state==='update'?<button className="primary-button" disabled={locked||checking} onClick={()=>void action('update_agent',{id:s.id,version:release.latestVersion})}><Download size={14}/>更新</button>:available&&state==='unknown'?<button className="quiet-button" disabled={locked||checking} onClick={()=>void action('update_agent',{id:s.id,version:release.latestVersion})}>安装最新版</button>:null}
        </>}</div></div>
        {working(s)?<progress aria-label={`${s.name} 安装进度`} max={s.total||undefined} value={s.total&&s.received!=null?s.received:undefined}/>:s.message?<p className={`agent-install-message ${s.phase==='failed'?'failed':''}`} role="status">{s.message}</p>:null}
      </section>;
    })}</div>
    <p className="agent-settings-footnote">保留已有登录和配置。新版在独立目录安装，测试通过后启用；更新失败时继续使用原版本。</p>
    <button className="agent-advanced-toggle" aria-expanded={advanced} onClick={()=>setAdvanced(v=>!v)}>高级选项<ChevronDown size={14} className={advanced?'expanded':''}/></button>
    {advanced?<div className="agent-advanced"><div className="settings-section">{items.map(s=><div className="agent-path-option" key={s.id}><span>{s.name}<small>{s.path||'自动检测'}</small></span><button className="quiet-button" disabled={!desktop||locked} onClick={()=>void browse(s.id)}><FolderOpen size={14}/>选择程序</button>{['configured','managed'].includes(s.source??'')?<button className="quiet-button" disabled={locked} onClick={()=>void action('configure_agent',{id:s.id,path:null}).then(()=>action('list_agents'))}>恢复自动</button>:null}<button className="quiet-button" disabled={!desktop||locked||checking||!releases[s.id]?.latestVersion||!!releases[s.id]?.error} onClick={()=>void action('install_agent',{id:s.id,repair:true,version:releases[s.id]?.latestVersion})}>修复安装</button></div>)}</div><label className="checkbox-label"><input type="checkbox" checked={loadMcp} disabled={locked} onChange={e=>setLoadMcp(e.target.checked)}/>加载 Codex 全局 MCP 服务</label><button className="quiet-button" disabled={locked||!desktop} onClick={()=>void saveMcp()}>保存选项</button></div>:null}
  </div>;
}
