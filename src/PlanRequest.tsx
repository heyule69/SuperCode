import { useRef, useState } from 'react';
import { ArrowRight, ListChecks, Pencil } from 'lucide-react';
import type { RpcEvent } from './types';
import { planText } from './planPresentation';
import { PlanDocument } from './PlanDocument';
import { CopyButton } from './CopyButton';
import './plan-request.css';

export default function PlanRequest({ request, respond, openFile }: { request: RpcEvent; respond: (result: unknown) => Promise<void>; openFile?: (path: string, line?: number) => void }) {
  const plan = planText(request.params.input) || planText(request.params);
  const [feedback, setFeedback] = useState('');
  const [busy, setBusy] = useState(false);
  const [submitted, setSubmitted] = useState<'execute' | 'revise' | null>(null);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const disabled = busy || submitted !== null;
  async function submit(execute: boolean) {
    if (pending.current || (execute && !plan)) return;
    pending.current = true;
    setBusy(true); setError('');
    try {
      await respond(execute ? { decision: 'accept' } : { decision: 'decline', feedback: feedback.trim() });
      setSubmitted(execute ? 'execute' : 'revise');
    } catch (e) { pending.current = false; setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  return <section className="plan-request" aria-label="执行计划确认" aria-busy={busy}>
    <header className="plan-request-header"><div><ListChecks size={16} /><strong>执行计划</strong><span>{submitted === 'execute' ? '已确认执行' : submitted === 'revise' ? '已要求修改' : '等待你确认'}</span></div>{plan ? <CopyButton iconOnly text={plan} label="复制完整计划" /> : null}</header>
    {plan ? <PlanDocument text={plan} openFile={openFile} /> : <div className="plan-request-empty" role="status">Agent 尚未提供完整计划，请先让它补充计划内容。</div>}
    <footer className="plan-request-footer">
      <label className="plan-request-feedback"><Pencil size={14} aria-hidden="true" /><textarea rows={1} value={feedback} maxLength={4000} aria-label="计划修改意见" placeholder="补充修改意见（可选）" disabled={disabled} onChange={event => setFeedback(event.target.value)} /></label>
      {error ? <p className="plan-request-error" role="alert">{error}</p> : null}
      <div className="plan-request-actions"><small>确认后，Agent 将按计划开始执行。</small><button type="button" className="plan-request-revise" disabled={disabled} onClick={() => void submit(false)}>继续修改</button><button type="button" className="plan-request-execute" disabled={disabled || !plan} onClick={() => void submit(true)}>{busy ? '正在提交…' : '执行计划'}<ArrowRight size={14} /></button></div>
    </footer>
  </section>;
}
