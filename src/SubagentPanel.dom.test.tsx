// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Conversation } from './Conversation';
import SubagentPanel from './SubagentPanel';
import { collectSubagents } from './subagents';
import { boundedSubagentMessages } from './useSubagentHistory';
import { call, subscribe } from './api';
import type { Message, RpcEvent } from './types';
vi.mock('./api',()=>({desktop:true,call:vi.fn(),subscribe:vi.fn()}));
let root:Root,node:HTMLDivElement;
const off=vi.fn();let event:((event:RpcEvent)=>void)|undefined;
const message=(id:string,role:string,text:string,kind='agentMessage',data:Message['data']=null):Message=>({id,seq:0,sessionId:'s',role,text,kind,data});
const spawn=message('spawn','tool','','collabAgentToolCall',{status:'completed',receiverThreadIds:['a','b'],agentsStates:{a:{status:'completed',agentNickname:'demo',message:'检查完成'},b:{status:'running',agentNickname:'reviewer'}}});
beforeEach(async()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  node=document.createElement('div');document.body.append(node);root=createRoot(node);
  vi.mocked(call).mockReset();vi.mocked(subscribe).mockReset();off.mockClear();event=undefined;
  vi.mocked(subscribe).mockImplementation(async callback=>{event=callback;return off;});
  await act(async()=>{await import('./Markdown');});
});
afterEach(async()=>{await act(async()=>root.unmount());node.remove();});
const props={sessionId:'s',agents:collectSubagents([spawn]),select:vi.fn(),close:vi.fn(),openFile:vi.fn(),showDiff:vi.fn(),expanded:true,expand:vi.fn()};
it('shows an async child as working until its correlated report arrives, without rendering SDK launch metadata',async()=>{
  const launch=message('task','tool','Async agent launched successfully.\nagentId: a (internal ID)','claudeToolCall',{tool:'Agent',status:'completed',subagentAsync:true,subagentStatus:'running',subagentResult:'',arguments:{description:'normal-demo'}});
  const draw=async(child:Message)=>act(async()=>root.render(<SubagentPanel {...props} agents={collectSubagents([child])} selected="task"/>));
  await draw(launch);
  expect(node.textContent).toContain('正在思考');
  expect(node.textContent).not.toContain('Async agent launched');
  expect(node.querySelector('[aria-label="复制回复"]')).toBeNull();
  const completed={...launch,data:{...launch.data,subagentStatus:'completed',subagentResult:'# 完整报告\n\n| 场景 | 结果 |\n|---|---|\n| 边界 | TypeError |\n\nCHILD_DONE',subagentDurationMs:40000}};
  await draw(completed);
  expect(node.textContent).toContain('完整报告'); expect(node.textContent).toContain('CHILD_DONE');
  expect(node.querySelector('table')).not.toBeNull();
  expect(node.querySelector('[aria-label="任务运行状态"]')).toBeNull();
  expect(call).not.toHaveBeenCalled();
});
it('keeps a completed child clickable while its parent continues thinking, without raw JSON or duplicate wait rows',async()=>{
  const open=vi.fn();const wait=message('wait','tool','','collabAgentToolCall',{status:'completed',agentsStates:{a:{status:'completed'},b:{status:'running'}}});
  await act(async()=>root.render(<Conversation messages={[message('run','activity','','runMarker',{status:'inProgress'}),spawn,wait]} sessionStatus="running" agentName="Codex" autoExpand={false} openFile={()=>{}} showDiff={()=>{}} openSubagent={open}/>));
  expect(node.querySelectorAll('.subagent-row')).toHaveLength(2);
  expect(node.textContent).toContain('Agent demo已完成');
  expect(node.textContent).toContain('正在思考');
  expect(node.textContent).not.toContain('agentsStates');
  await act(async()=>node.querySelector<HTMLButtonElement>('.subagent-row')!.click());
  expect(open).toHaveBeenCalledWith('a');
});
it('keeps subagent rows visible after parent completion',async()=>{
  await act(async()=>root.render(<Conversation messages={[spawn,message('final','assistant','汇总结果'),message('run','activity','','runMarker',{status:'completed'})]} sessionStatus="idle" agentName="Codex" autoExpand={false} openFile={()=>{}} showDiff={()=>{}} openSubagent={()=>{}}/>));
  expect(node.querySelectorAll('.subagent-row')).toHaveLength(2);
  expect(node.textContent).toContain('汇总结果');
});
it('replays live child completion over a late history snapshot and unsubscribes when closed',async()=>{
  let resolve!:(value:unknown)=>void;
  vi.mocked(call).mockImplementation(()=>new Promise(done=>{resolve=done;}) as any);
  await act(async()=>root.render(<SubagentPanel {...props} selected="a"/>));
  expect(call).toHaveBeenCalledWith('get_subagent_messages',{sessionId:'s',threadId:'a'});
  await act(async()=>{
    event!({method:'item/completed',params:{threadId:'a',turnId:'t',item:{id:'answer',type:'agentMessage',text:'完整的子代理回答',phase:'final_answer'}}});
    event!({method:'turn/completed',params:{threadId:'a',turn:{id:'t',status:'completed'}}});
    resolve({messages:[message('run-t','activity','','runMarker',{status:'inProgress',turnId:'t'})],status:'inProgress',turnId:'t',model:'child-model'});
  });
  expect(node.textContent).toContain('完整的子代理回答');
  expect(node.textContent).toContain('child-model');
  expect(node.querySelector('[aria-label="任务运行状态"]')).toBeNull();
  await act(async()=>root.render(<div/>));
  expect(off).toHaveBeenCalledOnce();
});
it('ignores another child and stale reads after switching the selected agent',async()=>{
  const pending:((value:unknown)=>void)[]=[];
  vi.mocked(call).mockImplementation(()=>new Promise(done=>pending.push(done)) as any);
  await act(async()=>root.render(<SubagentPanel {...props} selected="a"/>));
  await act(async()=>root.render(<SubagentPanel {...props} selected="b"/>));
  await act(async()=>{
    event!({method:'item/completed',params:{threadId:'a',turnId:'t',item:{id:'wrong',type:'agentMessage',text:'不属于当前子代理'}}});
    pending[0]({messages:[message('old','assistant','过期记录')],status:'completed'});
    pending[1]({messages:[message('current','assistant','当前子代理的回答')],status:'completed'});
  });
  expect(node.textContent).toContain('当前子代理的回答');
  expect(node.textContent).not.toContain('过期记录');
  expect(node.textContent).not.toContain('不属于当前子代理');
});
it('shows errors with retry and lets users return to a list of distinct agents',async()=>{
  vi.mocked(call).mockRejectedValue(new Error('记录暂时不可用'));
  await act(async()=>root.render(<SubagentPanel {...props} selected="a"/>));
  expect(node.textContent).toContain('记录暂时不可用');
  await act(async()=>node.querySelector<HTMLButtonElement>('.subagent-error button')!.click());
  expect(call).toHaveBeenCalledTimes(2);
  await act(async()=>node.querySelector<HTMLButtonElement>('[aria-label="返回子代理列表"]')!.click());
  expect(props.select).toHaveBeenCalledWith(null);
  await act(async()=>root.render(<SubagentPanel {...props} selected={null}/>));
  expect(node.querySelectorAll('.subagent-panel-list button')).toHaveLength(2);
});
it('uses the native Task result without starting or reading a new thread',async()=>{
  const agents=collectSubagents([message('task','tool','只读检查完成','claudeToolCall',{tool:'Task',status:'completed',arguments:{description:'检查',model:'actual-model'}})]);
  await act(async()=>root.render(<SubagentPanel {...props} agents={agents} selected="task"/>));
  expect(node.textContent).toContain('只读检查完成');
  expect(call).not.toHaveBeenCalled();
  expect(subscribe).not.toHaveBeenCalled();
});
it('keeps the child task and every completed execution step visible, with tool output available on demand',async()=>{
  const prompt='只读检查 README，然后验证边界情况。\n不要改写项目。';
  const agents=collectSubagents([{...spawn,data:{...spawn.data,prompt}}]);
  const steps=Array.from({length:5},(_,i)=>message(`read-${i}`,'tool',`read fixture-${i}.md\nPRIVATE_OUTPUT_${i}`,'commandExecution',{command:`read fixture-${i}.md`,status:'completed',commandActions:[{type:'read',path:`fixture-${i}.md`}]}));
  vi.mocked(call).mockResolvedValue({messages:[message('start','assistant','先检查输入边界。','agentMessage',{phase:'commentary'}),...steps,message('final','assistant','完整检查报告。','agentMessage',{phase:'final_answer'}),message('run','activity','','runMarker',{status:'completed',durationMs:17000})],status:'completed'});
  await act(async()=>root.render(<SubagentPanel {...props} agents={agents} selected="a"/>));
  expect(node.querySelector('[aria-label="子代理任务"]')?.textContent).toContain(prompt);
  expect(node.querySelector('.subagent-task details')).toBeNull();
  expect(node.textContent).toContain('先检查输入边界');
  expect(node.textContent).toContain('完整检查报告');
  expect(node.querySelectorAll('.activity-item')).toHaveLength(5);
  expect(node.querySelector('.turn-process-toggle')).toBeNull();
  expect(node.querySelector('.activity-group-summary')).toBeNull();
  expect(node.querySelector('.activity-toggle-more')).toBeNull();
  expect(node.querySelector('.activity-body')).toBeNull();
  for(let i=0;i<5;i++)expect(node.textContent).toContain(`fixture-${i}.md`);
  expect(node.textContent).not.toContain('PRIVATE_OUTPUT');
  await act(async()=>node.querySelector<HTMLButtonElement>('.activity-row')!.click());
  expect(node.textContent).toContain('PRIVATE_OUTPUT_0');
  expect(node.querySelectorAll('.activity-item')).toHaveLength(5);
});
it('retains live thinking, plan steps and tool steps when a child completes, without opening raw details',async()=>{
  vi.mocked(call).mockResolvedValue({messages:[message('run-t','activity','','runMarker',{status:'inProgress',turnId:'t'}),message('think','activity','检查边界输入','reasoning',{status:'inProgress'}),message('plan','activity','依次验证','executionPlan',{status:'inProgress',plan:[{step:'名称去空格',status:'completed'},{step:'非法数量抛错',status:'inProgress'}]})],status:'inProgress',turnId:'t'});
  await act(async()=>root.render(<SubagentPanel {...props} selected="b"/>));
  expect(node.querySelector('.activity-row .pending-text')?.textContent).toBe('思考中');
  expect(node.querySelector('[aria-label="执行计划：运行中"]')).not.toBeNull();
  expect(node.textContent).toContain('名称去空格');
  expect(node.textContent).toContain('非法数量抛错');
  await act(async()=>{
    event!({method:'item/completed',params:{threadId:'b',turnId:'t',item:{id:'tool',type:'commandExecution',command:'node check.mjs',status:'completed',aggregatedOutput:'RAW_CHECK_RESULT'}}});
    event!({method:'item/completed',params:{threadId:'b',turnId:'t',item:{id:'final',type:'agentMessage',text:'边界验证完成。',phase:'final_answer'}}});
    event!({method:'turn/completed',params:{threadId:'b',turn:{id:'t',status:'completed'}}});
    await new Promise(resolve=>setTimeout(resolve,70));
  });
  expect(node.textContent).toContain('边界验证完成');
  expect(node.textContent).toContain('已运行');
  expect(node.querySelectorAll('.activity-item')).toHaveLength(3);
  expect(node.querySelector('.turn-process-toggle')).toBeNull();
  expect(node.querySelectorAll('.activity-body')).toHaveLength(1);
  expect(node.querySelector('.activity-body .plan-steps')).not.toBeNull();
  expect(node.textContent).not.toContain('RAW_CHECK_RESULT');
});
it('bounds child display history to a small tail cache',()=>{
  expect(boundedSubagentMessages(Array.from({length:600},(_,i)=>message(`m${i}`,'assistant','内容')))).toHaveLength(200);
});
it('shows native forwarded Claude child step summaries before and after completion without raw tool details',async()=>{
  const data={tool:'Agent',status:'completed',subagentAsync:true,subagentStatus:'running',arguments:{description:'步骤检查',prompt:'只读检查 README'},subagentSteps:[{id:'read',type:'claudeToolCall',tool:'Read',arguments:{path:'README.md'},status:'inProgress'}]};
  const draw=async(d:typeof data & Record<string,unknown>)=>act(async()=>root.render(<SubagentPanel {...props} agents={collectSubagents([message('agent','tool','','claudeToolCall',d)])} selected="agent"/>));
  await draw(data);
  expect(node.querySelector('[aria-label="子代理任务"]')?.textContent).toContain('只读检查 README');
  expect(node.textContent).toContain('读取文件');
  expect(node.querySelector('.activity-row .pending-text')?.textContent).toBe('运行中');
  await draw({...data,subagentStatus:'completed',subagentResult:'检查完成。',subagentSteps:[{...data.subagentSteps[0],status:'completed'}]});
  expect(node.textContent).toContain('读取了文件');
  expect(node.textContent).toContain('README.md');
  expect(node.textContent).toContain('检查完成');
  expect(node.querySelector('.turn-process-toggle')).toBeNull();
  expect(node.querySelector('.activity-body')).toBeNull();
});
