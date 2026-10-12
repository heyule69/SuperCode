import { useRef, useState } from 'react';
import type { RpcEvent } from './types';
import { planText } from './planPresentation';
import { PlanCard } from './PlanCard';
import { PlanDecisionPrompt } from './PlanDecisionPrompt';
import './plan-request.css';

export default function PlanRequest({ request, respond, openFile }: { request: RpcEvent; respond: (result: unknown) => Promise<void>; openFile?: (path: string, line?: number) => void }) {
  const plan = planText(request.params.input) || planText(request.params);
  const [feedback, setFeedback] = useState('');
  const [busy, setBusy] = useState(false);
  const [submitted, setSubmitted] = useState<'execute' | 'revise' | null>(null);
  const [error, setError] = useState('');
  const pending = useRef(false);
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
    {plan ? <PlanCard text={plan} openFile={openFile} status={submitted === 'execute' ? '已确认执行' : submitted === 'revise' ? '已要求修改' : undefined}/> : <div className="plan-request-empty" role="status">Agent 尚未提供完整计划，请先让它补充计划内容。</div>}
    <PlanDecisionPrompt feedback={feedback} setFeedback={setFeedback} busy={busy} submitted={submitted} error={error} canExecute={!!plan} submit={decision => void submit(decision === 'execute')}/>
  </section>;
}
