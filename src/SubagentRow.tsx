import { createContext, useContext } from 'react';
import type { Subagent } from './subagents';
import { subagentLabel, subagentStateText, subagentVisual } from './subagents';
import './subagents.css';

export const SubagentContext = createContext<{ agents:Subagent[]; open?:(key:string)=>void }>({ agents:[] });
const shapes = [
  <g key="diamonds"><path d="m12 2 5 5-5 5-5-5Z"/><path d="m6 8 5 5-5 5-5-5Z"/><path d="m18 8 5 5-5 5-5-5Z"/><path d="m12 14 5 5-5 5-5-5Z"/></g>,
  <g key="petals"><circle cx="8" cy="8" r="5"/><circle cx="16" cy="8" r="5"/><circle cx="8" cy="16" r="5"/><circle cx="16" cy="16" r="5"/></g>,
  <g key="triangles"><path d="m12 2 5 9H7Z"/><path d="m6 12 5 9H1Z"/><path d="m18 12 5 9H13Z"/></g>,
  <g key="hexagon"><path d="m12 2 9 5v10l-9 5-9-5V7Z"/><path d="m3 7 9 5 9-5M12 12v10" fill="none" stroke="currentColor" strokeWidth="1.4"/></g>,
  <g key="star"><path d="m12 1 3 7 8 4-8 4-3 7-3-7-8-4 8-4Z"/><circle cx="12" cy="12" r="3" fill="currentColor" stroke="none"/></g>,
  <g key="tiles"><rect x="2" y="2" width="9" height="9" rx="2"/><rect x="13" y="2" width="9" height="9" rx="2"/><rect x="2" y="13" width="9" height="9" rx="2"/><rect x="13" y="13" width="9" height="9" rx="2"/></g>,
];
export function SubagentAvatar({ agentKey, visual:assigned }: { agentKey:string; visual?:Subagent['visual'] }) {
  const context=useContext(SubagentContext);
  const visual=assigned??context.agents.find(agent=>agent.key===agentKey)?.visual??subagentVisual(agentKey);
  return <svg className={`subagent-avatar color-${visual.color}`} data-subagent-shape={visual.shape} viewBox="0 0 24 24" aria-hidden="true" fill="currentColor" fillOpacity=".68" stroke="currentColor" strokeWidth=".65">{shapes[visual.shape]}</svg>;
}
export function SubagentRows({ messageId }: { messageId:string }) {
  const {agents,open}=useContext(SubagentContext);
  const rows = agents.filter(agent=>agent.anchorId===messageId);
  if (!rows.length) return null;
  return <div className="subagent-rows">{rows.map(agent=><button type="button" key={agent.key} data-subagent-key={agent.key} className={`subagent-row ${agent.status}`} disabled={!open} onClick={()=>open?.(agent.key)} aria-label={`查看 ${subagentLabel(agent)}，${subagentStateText[agent.status]}`}><SubagentAvatar agentKey={agent.key}/><span className={['running','waiting'].includes(agent.status)?'pending-text':undefined}><span className="subagent-row-name" title={agent.name}>{subagentLabel(agent)}</span><span>{subagentStateText[agent.status]}</span></span></button>)}</div>;
}
