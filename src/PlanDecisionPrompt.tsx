import { useContext, useLayoutEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { ArrowRight, Check, Pencil, Square, X } from 'lucide-react';
import { PlanDisplayContext } from './PlanDisplayContext';
import type { PlanDecision } from './planPresentation';
import './plan-request.css';

export function PlanDecisionPrompt({ feedback, setFeedback, busy, submitted, error, canExecute, submit }: {
  feedback: string; setFeedback: (value: string) => void; busy: boolean;
  submitted?: PlanDecision | null; error: string; canExecute: boolean;
  submit: (decision: PlanDecision) => void;
}) {
  const { decisionContainer } = useContext(PlanDisplayContext);
  const input = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    if (!input.current) return;
    input.current.style.height = 'auto';
    input.current.style.height = `${Math.min(88, input.current.scrollHeight)}px`;
  }, [feedback, decisionContainer]);
  const disabled = busy || !!submitted;
  const prompt = <footer className="plan-request-footer plan-decision" aria-label="实施计划" aria-busy={busy}>
    <header className="plan-decision-heading"><strong>{submitted === 'execute' ? '已确认执行' : submitted === 'revise' ? '已要求修改' : '实施此计划？'}</strong>
      <button type="button" className="icon-button" aria-label="关闭并继续修改计划" disabled={disabled} onClick={() => submit('revise')}><X size={15}/></button>
    </header>
    <button type="button" className="plan-request-execute plan-decision-option" disabled={disabled || !canExecute} onClick={() => submit('execute')}>
      {submitted === 'execute' ? <Check size={17}/> : <Square size={17}/>}<span>{busy ? '正在提交…' : '是，实施此计划'}</span><ArrowRight size={16}/>
    </button>
    {error ? <p className="plan-request-error" role="alert">{error}</p> : null}
    <div className="plan-decision-reply">
      <label className="plan-request-feedback"><Pencil size={15} aria-hidden="true"/><textarea ref={input} rows={1} value={feedback} maxLength={4000} aria-label="计划修改意见" placeholder="否，并说明需要如何调整" disabled={disabled} onChange={event => setFeedback(event.target.value)} onKeyDown={event => {
        if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && !event.repeat && feedback.trim()) { event.preventDefault(); submit('revise'); }
      }}/></label>
      <button type="button" className="plan-request-revise" disabled={disabled} onClick={() => submit('revise')}>{feedback.trim() ? '发送' : '跳过'}</button>
    </div>
  </footer>;
  return decisionContainer === undefined ? prompt : decisionContainer ? createPortal(prompt, decisionContainer) : null;
}
