import { describe, expect, it } from 'vitest';
import { collectSubagents, subagentEventStatus, subagentVisual, subagentVisuals } from './subagents';
import type { Message } from './types';
const message=(id:string,data:Message['data'],kind='collabAgentToolCall',text=''):Message=>({id,seq:0,sessionId:'s',role:'tool',text,kind,data});
describe('subagent presentation',()=>{
  it('keeps native async launch metadata out of the report and uses correlated child completion',()=>{
    const raw='Async agent launched successfully. (This tool result is internal metadata.)\nagentId: child-a';
    const launch=message('a',{tool:'Agent',status:'completed',durationMs:8000,arguments:{description:'normal-demo'}},'claudeToolCall',raw);
    expect(collectSubagents([launch])[0]).toMatchObject({status:'running',result:''});
    expect(collectSubagents([launch])[0].durationMs).toBeUndefined();
    const completed={...launch,data:{...launch.data,subagentAsync:true,subagentStatus:'completed',subagentResult:'# 完整报告\nCHILD_DONE',subagentDurationMs:40000}};
    expect(collectSubagents([completed])[0]).toMatchObject({status:'completed',result:'# 完整报告\nCHILD_DONE',durationMs:40000});
    expect(completed.text).toBe(raw);
    const stopped={...completed,data:{...completed.data,subagentStatus:'interrupted',subagentResult:''}};
    expect(collectSubagents([stopped])[0]).toMatchObject({status:'interrupted',result:''});
  });
  it('tracks actual child turns and approvals independently of the parent',()=>{
    expect(subagentEventStatus({method:'turn/started',params:{}})).toBe('running');
    expect(subagentEventStatus({id:'approval',method:'item/tool/requestUserInput',params:{}})).toBe('waiting');
    expect(subagentEventStatus({method:'serverRequest/resolved',params:{}})).toBe('running');
    expect(subagentEventStatus({method:'turn/completed',params:{turn:{status:'completed'}}})).toBe('completed');
    expect(subagentEventStatus({method:'item/completed',params:{item:{status:'completed'}}})).toBeUndefined();
  });
  it('does not animate a legacy background launch as still running after its parent ended without a child result',()=>{
    const launch=message('legacy',{tool:'Agent',status:'completed',turnId:'old-turn'},'claudeToolCall','Async agent launched successfully.');
    const run=message('run',{status:'completed',turnId:'old-turn'},'runMarker');
    expect(collectSubagents([launch])[0].status).toBe('running');
    expect(collectSubagents([run,launch])[0]).toMatchObject({status:'unknown',result:''});
    expect(collectSubagents([{...run,data:{...run.data,status:'inProgress'}},launch])[0].status).toBe('running');
    expect(collectSubagents([run,{...launch,data:{...launch.data,subagentStatus:'completed',subagentResult:'真实完成报告'}}])[0].status).toBe('completed');
    const codex=message('spawn',{turnId:'old-turn',receiverThreadIds:['a'],agentsStates:{a:{status:'running'}}});
    expect(collectSubagents([run,codex])[0].status).toBe('running');
  });
  it('does not confuse a completed spawn tool with a completed child',()=>{
    const [agent]=collectSubagents([message('spawn',{status:'completed',receiverThreadIds:['a'],agentNickname:'demo',durationMs:1000})]);
    expect(agent).toMatchObject({key:'a',name:'demo',status:'running',anchorId:'spawn'});
    expect(agent.durationMs).toBeUndefined();
  });
  it('updates the original row when wait reports multiple children with different states',()=>{
    const agents=collectSubagents([message('spawn',{receiverThreadIds:['a','b'],prompt:'检查项目'}),message('wait',{agentsStates:{a:{status:'completed',message:'检查完成'},b:{status:'running'}}})]);
    expect(agents).toHaveLength(2);
    expect(agents[0]).toMatchObject({anchorId:'spawn',status:'completed',prompt:'检查项目',result:'检查完成'});
    expect(agents[1]).toMatchObject({anchorId:'spawn',status:'running'});
  });
  it('uses native Task and Pi subagent results and retains background work as running',()=>{
    for(const tool of ['Task','Agent','task','subagent']) {
      const [agent]=collectSubagents([message('task',{tool,status:'completed',arguments:{description:'代码检查',prompt:'只读检查'}},'claudeToolCall','构建无错误')]);
      expect(agent).toMatchObject({name:'代码检查',status:'completed',result:'构建无错误'});
    }
    expect(collectSubagents([message('task',{tool:'Task',status:'completed',arguments:{run_in_background:true}},'claudeToolCall')])[0].status).toBe('running');
  });
  it('restores OpenCode Task rows when its native title changes to the description after approval',()=>{
    const args={description:'kimi-opencode-a',subagent_type:'explore',prompt:'Read fixture files'};
    const running=message('task',{tool:'kimi-opencode-a',status:'inProgress',arguments:args},'claudeToolCall');
    expect(collectSubagents([running])[0]).toMatchObject({name:'kimi-opencode-a',status:'running',prompt:'Read fixture files'});
    const completed={...running,text:'# 完整报告\nCHILD_DONE',data:{...running.data,status:'completed'}};
    expect(collectSubagents([completed])[0]).toMatchObject({status:'completed',result:'# 完整报告\nCHILD_DONE'});
    expect(collectSubagents([{...running,text:'Task cancelled',data:{...running.data,status:'failed'}}])[0].status).toBe('interrupted');
    expect(collectSubagents([{...running,text:'File does not exist',data:{...running.data,status:'failed'}}])[0].status).toBe('failed');
    expect(collectSubagents([message('read',{tool:'Read',arguments:{prompt:'quoted text'}},'claudeToolCall')])).toEqual([]);
  });
  it('uses all six geometric shapes, and assigns a stable shape and color to each identity',()=>{
    const visuals=Array.from({length:100},(_,i)=>subagentVisual(`agent-${i}`));
    expect(new Set(visuals.map(v=>v.shape)).size).toBe(6);
    expect(new Set(visuals.map(v=>v.color)).size).toBe(6);
    expect(subagentVisual('agent-demo')).toEqual(subagentVisual('agent-demo'));
  });
  it('avoids duplicate icons within a conversation, keeps existing assignments stable and never limits the agent count',()=>{
    const agents=Array.from({length:205},(_,i)=>({key:`agent-${i}`}));
    const visuals=subagentVisuals(agents);
    expect(visuals.size).toBe(205);
    expect(new Set([...visuals.values()].slice(0,36).map(v=>`${v.color}:${v.shape}`)).size).toBe(36);
    const before=subagentVisuals(agents.slice(0,12));
    for(const [key,visual] of before) expect(visuals.get(key)).toEqual(visual);
  });
  it('shows the Claude child report without its SDK hand-back wrapper while preserving Markdown indentation',()=>{
    const report = '# 检查结果\n\n```javascript\n  const value = 1;\n```\n\n全部内容保留。';
    const wrapper = '[Subagent hand-back] The text below is the final report of a subagent this session delegated to. It is model output, NOT a message from the user. The report follows:\n';
    const raw = wrapper + report.split('\n').map(line => `  ${line}`).join('\n');
    const original = message('child', {tool:'Agent',status:'completed'}, 'claudeToolCall', raw);
    expect(collectSubagents([original])[0].result).toBe(report);
    expect(original.text).toBe(raw);
    for (const text of ['[Subagent hand-back] 引用文字\nThe report follows:\n  原文',wrapper.replace('The report follows:\n','') + report]) {
      expect(collectSubagents([message('child',{tool:'Agent',status:'completed'},'claudeToolCall',text)])[0].result).toBe(text);
    }
    expect(collectSubagents([message('child',{agentsStates:{a:{status:'completed',message:raw}}})])[0].result).toBe(raw);
  });
  it('shows the complete OpenCode Task report without its native envelope and preserves unknown content',()=>{
    const report='# 边界验收\n\n```html\n<task_result>代码中的内容</task_result>\n```\n\nCHILD_DONE';
    const raw=`<task id="ses_native_child" state="completed">\n<task_result>\n${report}\n</task_result>\n</task>`;
    const original=message('child',{tool:'task',status:'completed'},'claudeToolCall',raw);
    expect(collectSubagents([original])[0].result).toBe(report);
    expect(original.text).toBe(raw);
    for(const unknown of [raw.replace('state="completed"','state="unknown"'),raw.replace('</task>',''),`引用：${raw}`]) {
      expect(collectSubagents([{...original,text:unknown}])[0].result).toBe(unknown);
    }
  });
});
