import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { ArrowRight, Check, ChevronLeft, MessageCircleQuestion, Pencil, X } from 'lucide-react';
import type { RpcEvent } from './types';
import './question-request.css';

type Question = { id: string; question: string; header?: string; multiSelect?: boolean; isSecret?: boolean; options?: { label: string; description?: string }[] };
type Props = { request: RpcEvent; respond: (result: unknown) => Promise<void> };
type Answers = Record<string, string>;
type Choices = Record<string, string[]>;

export default function QuestionRequest({ request, respond }: Props) {
  const questions: Question[] = request.params.questions ?? [];
  const [page, setPage] = useState(0);
  const [answers, setAnswers] = useState<Answers>({});
  const [choices, setChoices] = useState<Choices>({});
  const [busy, setBusy] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const panel = useRef<HTMLElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const id = useId();
  const question = questions[page];
  const answered = (q: Question, text = answers, selected = choices) => Boolean(text[q.id]?.trim() || selected[q.id]?.length);
  const complete = questions.length > 0 && questions.every(q => answered(q));
  const last = page === questions.length - 1;
  const disabled = busy || submitted;
  // Pi's native select dialog only accepts one of its advertised options.
  const custom = request.params.nativeRequest?.method !== 'select';

  useEffect(() => { panel.current?.focus({ preventScroll: true }); if (content.current) content.current.scrollTop = 0; }, [request.id, page]);
  useEffect(() => {
    if (!input.current) return;
    input.current.style.height = 'auto';
    input.current.style.height = `${Math.min(72, input.current.scrollHeight)}px`;
  }, [page, answers]);

  function choose(label: string) {
    if (!question || pending.current) return;
    setError('');
    if (question.multiSelect) {
      setChoices(old => {
        const selected = old[question.id] ?? [];
        return { ...old, [question.id]: selected.includes(label) ? selected.filter(v => v !== label) : [...selected, label] };
      });
      return;
    }
    const nextChoices = { ...choices, [question.id]: [label] };
    const nextAnswers = { ...answers, [question.id]: '' };
    setChoices(nextChoices); setAnswers(nextAnswers);
    // Pass this click's answer directly: React state has not committed yet.
    next(nextAnswers, nextChoices);
  }
  async function submit(skip = false, text = answers, selected = choices) {
    if (pending.current || (!skip && !(questions.length > 0 && questions.every(q => answered(q, text, selected))))) return;
    pending.current = true;
    setBusy(true); setError('');
    try {
      const claude = request.params.toolName === 'AskUserQuestion' || String(request.id).startsWith('claude:');
      const result = skip && claude ? { decision: 'decline' } : { answers: Object.fromEntries(questions.map(q => [q.id, { answers: skip ? [] : [
        ...(selected[q.id] ?? []), ...(text[q.id]?.trim() ? [text[q.id].trim()] : []),
      ] }])) };
      await respond(result);
      setSubmitted(true);
    } catch (e) { pending.current = false; setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  function next(text = answers, selected = choices) {
    if (pending.current || !question || !answered(question, text, selected)) return;
    if (last) {
      const missing = questions.findIndex(q => !answered(q, text, selected));
      if (missing >= 0) setPage(missing);
      else void submit(false, text, selected);
    } else setPage(page + 1);
  }
  function keyDown(e: KeyboardEvent<HTMLElement>) {
    if (e.altKey || e.ctrlKey || e.metaKey || e.nativeEvent.isComposing || pending.current) return;
    const editing = (e.target as HTMLElement).matches('textarea, input, [contenteditable=true]');
    if (!editing && /^[1-9]$/.test(e.key)) {
      if (e.repeat) return;
      const option = question?.options?.[Number(e.key) - 1];
      if (option) { e.preventDefault(); choose(option.label); }
    } else if (e.key === 'Enter' && !e.shiftKey && (editing || e.target === panel.current)) {
      e.preventDefault(); next();
    }
  }

  return <section ref={panel} className="question-request" tabIndex={-1} aria-label="Agent 提问" aria-busy={busy} onKeyDown={keyDown}>
    <header className="question-request-header"><span><MessageCircleQuestion size={16} />问题</span>
      <div>{questions.length > 1 ? <span className="question-request-page" aria-live="polite">{page + 1} / {questions.length}</span> : null}
        <button type="button" className="question-request-close" aria-label="跳过并关闭提问" disabled={disabled} onClick={() => void submit(true)}><X size={16} /></button></div>
    </header>
    {question ? <div ref={content} className="question-request-content" role="group" aria-labelledby={`${id}-title`}>
      <div className="question-request-heading">{questions.length > 1 && question.header ? <span>{question.header}</span> : null}
        <h3 id={`${id}-title`}>{question.question}</h3>{question.multiSelect ? <small>可多选</small> : null}</div>
      <div className="question-request-options">{(question.options ?? []).map((option, index) => {
        const selected = choices[question.id]?.includes(option.label) ?? false;
        return <button key={option.label} type="button" disabled={disabled} aria-pressed={selected} className={`question-request-option${selected ? ' is-selected' : ''}`} onClick={event => { if (event.detail <= 1) choose(option.label); }}>
          <span className="question-request-number" aria-hidden="true">{selected ? <Check size={12} /> : index + 1}</span>
          <span className="question-request-option-text"><strong>{option.label}</strong>{option.description ? <small>{option.description}</small> : null}</span>
          <ArrowRight className="question-request-option-arrow" size={15} aria-hidden="true" />
        </button>;
      })}</div>
    </div> : <p role="alert" className="question-request-error">Agent 没有提供可回答的问题，可以跳过后继续。</p>}
    {error ? <p className="question-request-error" role="alert">{error}</p> : null}
    <footer className="question-request-footer">
      {custom && question ? <label className="question-request-custom"><Pencil size={15} aria-hidden="true" />
        <textarea ref={input} rows={1} aria-label={`自定义回答：${question.question}`} disabled={disabled} placeholder="或自行撰写回复" value={answers[question.id] ?? ''} onChange={e => {
          setAnswers(old => ({ ...old, [question.id]: e.target.value }));
          if (!question.multiSelect) setChoices(old => ({ ...old, [question.id]: [] }));
        }} /></label> : <div className="question-request-footer-space" />}
      <div className="question-request-actions">
        {page > 0 ? <button type="button" className="question-request-back" aria-label="上一题" disabled={disabled} onClick={() => setPage(page - 1)}><ChevronLeft size={16} /></button> : null}
        <button type="button" className="question-request-skip" disabled={disabled} onClick={() => void submit(true)}>跳过</button>
        <button type="button" className="question-request-send" disabled={disabled || !question || (last ? !complete : !answered(question))} onClick={() => next()}>{busy ? '发送中…' : submitted ? '已回答' : last ? '发送' : '下一题'}</button>
      </div>
    </footer>
  </section>;
}
