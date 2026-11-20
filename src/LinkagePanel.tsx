import { AgentIcon } from './AgentIcon';
import { Conversation } from './Conversation';
import { Terminal } from 'lucide-react';
import type { LinkageState } from './useLinkage';
import type { OpenPath } from './ResourceMenu';
import './linkage.css';

const names:Record<string,string>={idle:'已就绪',starting:'正在启动',running:'正在执行',waiting:'等待确认',failed:'执行失败',interrupted:'已停止'};
export default function LinkagePanel({state,hidden,projectId,openFile,showDiff,openPath}: {
  state:LinkageState;hidden:boolean;projectId:string;openFile:(path:string,line?:number,sessionId?:string)=>void;
  showDiff:(name:string,diff:string)=>void;openPath:OpenPath;
}){
  const worker=state.worker;
  return <section className="linkage-live-panel" hidden={hidden} aria-label="联动执行记录">
    <header><AgentIcon agent={worker?.agent??state.configuration?.executor.agentId??''}/><div><strong>{worker?.agent??state.configuration?.executor.agentId??'执行 Agent'}</strong><small>{worker?.model??state.configuration?.executor.modelId}</small></div><span>{worker?names[worker.status]??worker.status:'尚未派发'}</span></header>
    {worker?<div className="linkage-live-scroll"><Conversation messages={state.messages} agentName={worker.agent} autoExpand={false} showDiff={showDiff}
      openFile={(path,line)=>openFile(path,line,worker.id)} openPath={openPath} projectId={projectId} sessionId={worker.id} sessionStatus={worker.status} activeTurnId={worker.turnId}/></div>
      :<div className="linkage-live-empty"><Terminal size={22}/><p>主控派发任务后，执行记录会显示在这里。</p></div>}
  </section>;
}
