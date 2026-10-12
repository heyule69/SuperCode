import { useRef, useState } from 'react';
import { ArrowRight, ListChecks, Pencil } from 'lucide-react';
import { PlanDocument } from './PlanDocument';
import type { PlanDecision } from './planPresentation';

export default function ProposedPlan({ text, streaming, respond, openFile }: {
  text: string; streaming: boolean; respond?: (decision: PlanDecision, feedback: string) => Promise<void>;
  openFile: (path: string, line?: number) => void;
}) {
  const [feedback, setFeedback] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [submitted, setSubmitted] = useState<PlanDecision>();
  const [error, setError] = useState('');
  const pending = useRef(false);
  async function submit(decision: PlanDecision) {
    if (!respond || pending.current || streaming || !text.trim()) return;
    pending.current = true; setSubmitting(true); setError('');
    try { await respond(decision, decision === 'revise' ? feedback.trim() : ''); setSubmitted(decision); }
    catch (e) { pending.current = false; setError(e instanceof Error ? e.message : String(e)); }
    finally { setSubmitting(false); }
  }
  if (streaming) return <div className="plan-generation" aria-label="正在生成计划"><PlanDocument text={text} streaming openFile={openFile}/></div>;
  const disabled = submitting || !!submitted;
  return <section className="plan-request" aria-label="原生计划文档" aria-busy={submitting}>
    <header className="plan-request-header"><div><ListChecks size={16}/><strong>{respond ? '执行计划' : '计划文档'}</strong><span>{submitted === 'execute' ? '已确认执行' : submitted === 'revise' ? '已要求修改' : respond ? '等待你确认' : ''}</span></div></header>
    <PlanDocument text={text} openFile={openFile}/>
    {respond ? <footer className="plan-request-footer">
      <label className="plan-request-feedback"><Pencil size={14}/><textarea rows={1} maxLength={4000} value={feedback} aria-label="计划修改意见" placeholder="补充修改意见（可选）" disabled={disabled} onChange={e=>setFeedback(e.target.value)}/></label>
      {error ? <p className="plan-request-error" role="alert">{error}</p> : null}
      <div className="plan-request-actions"><small>确认后，Agent 将按计划开始执行。</small><button type="button" className="plan-request-revise" disabled={disabled} onClick={()=>void submit('revise')}>继续修改</button><button type="button" className="plan-request-execute" disabled={disabled || !text.trim()} onClick={()=>void submit('execute')}>{submitting ? '正在提交…' : '执行计划'}<ArrowRight size={14}/></button></div>
    </footer> : null}
  </section>;
}
