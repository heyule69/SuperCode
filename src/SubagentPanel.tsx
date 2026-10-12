import { useEffect, useState, type ReactNode } from 'react';
import { ArrowLeft, Maximize2, Minimize2, Users, X } from 'lucide-react';
import { Conversation } from './Conversation';
import { SubagentAvatar } from './SubagentRow';
import { subagentLabel, subagentStateText, subagentStatus, type Subagent, type SubagentStatus } from './subagents';
import { useSubagentHistory } from './useSubagentHistory';
import type { Message } from './types';
import type { OpenPath } from './ResourceMenu';
import './subagents.css';

const sessionStatus=(status:string|undefined)=>status==='inProgress'?'running':status==='completed'?'idle':status;
function Detail({agent,back,onStatus,sessionId,projectId,openFile,showDiff,openPath}: {agent:Subagent;back:()=>void;onStatus?:(key:string,status:SubagentStatus)=>void;sessionId:string;projectId?:string;openFile:(path:string,line?:number)=>void;showDiff:(name:string,text:string)=>void;openPath?:OpenPath}) {
  const [revision,setRevision]=useState(0);
  const history=useSubagentHistory(sessionId,agent.threadId,revision);
  useEffect(()=>{if(history.status&&!history.loading)onStatus?.(agent.key,subagentStatus(history.status));},[history.status,history.loading,agent.key,onStatus]);
  const running=['running','waiting'].includes(agent.status);
  const fallback:Message[]=[...(agent.steps??[]),...(agent.result?[{seq:0,id:`result-${agent.key}`,sessionId,role:'assistant',kind:'agentMessage',text:agent.result,data:{status:running?'inProgress':'completed',phase:'final_answer'} }]:[])];
  if(!agent.threadId && agent.durationMs!=null)fallback.unshift({seq:0,id:`run-${agent.key}`,sessionId,role:'activity',kind:'runMarker',text:'',data:{status:running?'inProgress':agent.status,durationMs:agent.durationMs}});
  const messages=history.messages.length?history.messages:fallback;
  const model=[history.model??agent.model,history.effort??agent.effort].filter(Boolean).join(' · ');
  const status=agent.status==='waiting'?'waiting':sessionStatus(history.status)??(agent.status==='completed'?'idle':agent.status);
  return <>
    <div className="subagent-panel-heading"><button className="icon-button" aria-label="返回子代理列表" onClick={back}><ArrowLeft size={17}/></button><SubagentAvatar agentKey={agent.key} visual={agent.visual}/><strong title={subagentLabel(agent)}>{subagentLabel(agent)}</strong>{model?<span className="subagent-model">{model}</span>:null}</div>
    <div className="subagent-panel-scroll">
    {agent.prompt?<section className="subagent-task" aria-label="子代理任务"><h3>任务</h3><p>{agent.prompt}</p></section>:null}
    {history.loading?<div className="working-indicator" role="status"><span className="pending-text">正在加载子代理记录…</span></div>:null}
    {history.error?<div className="subagent-error" role="status"><span>无法加载执行过程：{history.error}</span><button className="quiet-button" onClick={()=>setRevision(v=>v+1)}>重试</button></div>:null}
    <div className="messages"><Conversation messages={messages} agentName={subagentLabel(agent)} sessionStatus={status} activeTurnId={history.turnId} autoExpand={false} processDisplay="timeline" showDiff={showDiff} openFile={openFile} openPath={openPath} projectId={projectId} sessionId={sessionId}/></div>
    {!messages.length&&!running&&!history.loading?<p className="subagent-empty">{subagentStateText[agent.status]}，引擎未返回可展示的结果。</p>:null}
    {!agent.threadId&&!agent.result&&running?<p className="subagent-empty">等待引擎返回子代理结果…</p>:null}
    {history.truncated||agent.stepsTruncated?<p className="subagent-empty">正在显示最近的执行记录。</p>:null}
    </div>
  </>;
}
export default function SubagentPanel({agents,selected,select,close,onStatus,sessionId,projectId,openFile,showDiff,openPath,expanded,expand,resizeHandle}: {agents:Subagent[];selected:string|null;select:(key:string|null)=>void;close:()=>void;onStatus?:(key:string,status:SubagentStatus)=>void;sessionId:string;projectId?:string;openFile:(path:string,line?:number)=>void;showDiff:(name:string,text:string)=>void;openPath?:OpenPath;expanded:boolean;expand:()=>void;resizeHandle?:ReactNode}) {
  const agent=agents.find(value=>value.key===selected);
  return <aside className="subagent-panel" aria-label="子智能体">
    {resizeHandle}
    <div className="subagent-panel-tabs"><div className="subagent-panel-tab"><Users size={16}/><span>子智能体</span><button className="icon-button" aria-label="关闭子智能体" onClick={close}><X size={15}/></button></div><button className="icon-button" aria-label={expanded?'收窄子代理面板':'展开子代理面板'} onClick={expand}>{expanded?<Minimize2 size={16}/>:<Maximize2 size={16}/>}</button></div>
    {agent?<Detail key={agent.key} agent={agent} back={()=>select(null)} onStatus={onStatus} sessionId={sessionId} projectId={projectId} openFile={openFile} showDiff={showDiff} openPath={openPath}/>:<><div className="subagent-panel-heading"><strong>子代理 · {agents.length}</strong></div><div className="subagent-panel-scroll"><div className="subagent-panel-list">{agents.map(agent=><button key={agent.key} onClick={()=>select(agent.key)}><SubagentAvatar agentKey={agent.key} visual={agent.visual}/><span>{subagentLabel(agent)}</span><small className={['running','waiting'].includes(agent.status)?'pending-text':undefined}>{subagentStateText[agent.status]}</small></button>)}</div></div></>}
  </aside>;
}
