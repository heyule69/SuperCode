import { Check, Circle } from 'lucide-react';
import type { LinkageTask } from './useLinkage';

const names: Record<string,string> = { working:'继续执行', needs_review:'待主控验收', completed:'已验收', blocked:'遇到阻碍', waiting_user:'等你确认', cancelled:'已停止' };
export function LinkageChecklist({ task }: { task: LinkageTask }) {
  return <section className="linkage-task-checklist" aria-label="任务验收清单">
    <header><strong>{names[task.state]}</strong><span>{task.steps.filter(step=>step.done).length} / {task.steps.length}</span></header>
    <ol>{task.steps.map((step,index)=><li key={index} className={step.done?'done':''} title={step.evidence}>
      {step.done?<Check size={12}/>:<Circle size={10}/>}<span>{step.text}</span>
    </li>)}</ol>
    {task.reason?<p>{task.reason}</p>:null}
    {task.reviewError?<p role="status">主控处理未完成：{task.reviewError}</p>:null}
    {task.executionError?<p role="status">执行未完成：{task.executionError}</p>:null}
    {task.state==='blocked'&&task.attempts?.length?<details><summary>已尝试的方法</summary><ul>{task.attempts.map((attempt,index)=><li key={index}>{attempt}</li>)}</ul></details>:null}
  </section>;
}
