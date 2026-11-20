import { createContext, useContext } from 'react';
import type { Subagent } from './subagents';
import { subagentLabel, subagentStateText, subagentVisual } from './subagents';
import './subagents.css';

export const SubagentContext = createContext<{ agents:Subagent[]; open?:(key:string)=>void }>({ agents:[] });
const expressions = [
  <path key="eyes" d="M8.5 11.5v2m7-2v2"/>,
  <path key="smile" d="M7.8 12.3q1-1.5 2 0m4.4 0q1-1.5 2 0"/>,
  <path key="calm" d="M8.4 12.3h1.2m4.8 0h1.2"/>,
  <g key="wink"><circle cx="8.8" cy="12.4" r=".85" fill="currentColor" stroke="none"/><path d="M14.5 11.5v1.8"/></g>,
  <path key="curious" d="M8.5 11.5v2m6-1.2h1.5"/>,
  <g key="dots"><circle cx="8.7" cy="12.3" r=".9" fill="currentColor" stroke="none"/><circle cx="15.3" cy="12.3" r=".9" fill="currentColor" stroke="none"/></g>,
];
export function SubagentAvatar({ agentKey, visual:assigned }: { agentKey:string; visual?:Subagent['visual'] }) {
  const context=useContext(SubagentContext);
  const visual=assigned??context.agents.find(agent=>agent.key===agentKey)?.visual??subagentVisual(agentKey);
  return <svg className={`subagent-avatar color-${visual.color}`} data-subagent-shape={visual.shape} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.55" strokeLinecap="round" strokeLinejoin="round"><rect x="4.25" y="6.5" width="15.5" height="13.25" rx="4.5" fill="currentColor" fillOpacity=".055"/><path d="M12 3.5v3M2 11v4m20-4v4"/>{expressions[visual.shape]}</svg>;
}
export function SubagentRows({ messageId }: { messageId:string }) {
  const {agents,open}=useContext(SubagentContext);
  const rows = agents.filter(agent=>agent.anchorId===messageId);
  if (!rows.length) return null;
  return <div className="subagent-rows">{rows.map(agent=><button type="button" key={agent.key} data-subagent-key={agent.key} className={`subagent-row ${agent.status}`} disabled={!open} onClick={()=>open?.(agent.key)} aria-label={`查看 ${subagentLabel(agent)}，${subagentStateText[agent.status]}`}><SubagentAvatar agentKey={agent.key}/><span className={['running','waiting'].includes(agent.status)?'pending-text':undefined}><span className="subagent-row-name" title={agent.name}>{subagentLabel(agent)}</span><span>{subagentStateText[agent.status]}</span></span></button>)}</div>;
}
