import { useRef, useState } from 'react';
import { PlanCard } from './PlanCard';
import { PlanDecisionPrompt } from './PlanDecisionPrompt';
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
  return <section className={streaming ? 'plan-generation' : 'plan-request'} aria-label="原生计划文档" aria-busy={submitting}>
    <PlanCard text={text} streaming={streaming} openFile={openFile} status={submitted === 'execute' ? '已确认执行' : submitted === 'revise' ? '已要求修改' : undefined}/>
    {respond && !streaming ? <PlanDecisionPrompt feedback={feedback} setFeedback={setFeedback} busy={submitting} submitted={submitted} error={error} canExecute={!!text.trim()} submit={decision => void submit(decision)}/> : null}
  </section>;
}
