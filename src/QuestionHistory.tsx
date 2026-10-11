import { Check } from 'lucide-react';
import { CopyButton } from './CopyButton';
import type { Message } from './types';
import './question-history.css';

type Option = { label: string; description: string };
type Question = { id: string; question: string; header: string; multiSelect: boolean; secret: boolean; options: Option[]; answers: string[] };

function parse(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  if (value.length > 128 * 1024) return undefined;
  try { return JSON.parse(value); } catch { return undefined; }
}
const record = (value: unknown): Record<string, unknown> | undefined => value != null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
const text = (value: unknown) => typeof value === 'string' ? value : '';

function answerList(value: unknown): string[] {
  if (typeof value === 'string') return value.trim() ? [value] : [];
  if (Array.isArray(value)) return value.filter((answer): answer is string => typeof answer === 'string' && !!answer.trim());
  const answers = record(value)?.answers;
  return Array.isArray(answers) ? answerList(answers) : [];
}

/** Read stored tool data without exposing its JSON protocol in the conversation. */
function history(message: Message) {
  const data = message.data ?? {};
  const args = record(parse(data.arguments));
  const streamed = record(parse(data.inputText));
  const input = Array.isArray(args?.questions) && args.questions.length ? args : streamed ?? args;
  const parsedOutput = parse(message.text);
  const result = record(parse(data.result)) ?? record(parsedOutput);
  const rawQuestions = input?.questions ?? result?.questions;
  const questions: Question[] = Array.isArray(rawQuestions) ? rawQuestions.flatMap((raw, index) => {
    const q = record(raw);
    if (!q || !text(q.question).trim()) return [];
    const id = text(q.id) || String(index);
    let answers: string[] = [];
    for (const source of [result?.answers, input?.answers]) {
      const map = record(source);
      if (!map) continue;
      for (const key of [text(q.question), id]) {
        if (Object.hasOwn(map, key)) answers = answerList(map[key]);
        if (answers.length) break;
      }
      if (answers.length) break;
    }
    const options: Option[] = Array.isArray(q.options) ? q.options.flatMap(rawOption => {
      const option = record(rawOption);
      return option && text(option.label).trim() ? [{ label: text(option.label), description: text(option.description) }] : [];
    }) : [];
    return [{ id, question: text(q.question), header: text(q.header), multiSelect: q.multiSelect === true, secret: q.isSecret === true, options, answers }];
  }) : [];
  // Text tool results can include explanations or a cancellation reason. Unknown
  // structured results stay out of the readable history, including partial JSON.
  const output = questions.some(q => q.secret) || questions.some(q => q.answers.length) ? '' : typeof parsedOutput === 'string' ? parsedOutput : parsedOutput === undefined && !/^\s*[\[{]/.test(message.text) ? message.text.slice(0, 128 * 1024) : '';
  return { questions, output };
}

export function QuestionHistory({ message }: { message: Message }) {
  const { questions, output } = history(message);
  const copyText = [...questions.map(q => [q.header, q.question, q.multiSelect ? '可多选' : '', ...q.options.map((option, index) => `${index + 1}. ${option.label}${option.description ? `\n${option.description}` : ''}`), q.answers.length ? `你的回答：${q.secret ? '内容已隐藏' : q.answers.join('、')}` : ''].filter(Boolean).join('\n')), output].filter(Boolean).join('\n\n');
  return <section className="question-history" aria-label="提问详情">
    {questions.length ? questions.map((q, index) => <div className="question-history-item" key={`${q.id}-${index}`}>
      {q.header || q.multiSelect || questions.length > 1 ? <div className="question-history-meta">{questions.length > 1 ? <span>问题 {index + 1} / {questions.length}</span> : null}{q.header ? <span>{q.header}</span> : null}{q.multiSelect ? <span>可多选</span> : null}</div> : null}
      <h4>{q.question}</h4>
      {q.options.length ? <ol className="question-history-options">{q.options.map((option, i) => {
        const selected = !q.secret && q.answers.includes(option.label);
        return <li key={i} className={selected ? 'is-selected' : undefined}><span className="question-history-number" aria-hidden="true">{selected ? <Check size={12} /> : i + 1}</span><div><strong>{option.label}</strong>{option.description ? <p>{option.description}</p> : null}</div>{selected ? <span className="sr-only">已选择</span> : null}</li>;
      })}</ol> : null}
      {q.answers.length ? <div className="question-history-answer"><span>你的回答</span><p>{q.secret ? '已回答（内容已隐藏）' : q.answers.join('、')}</p></div> : null}
    </div>) : <p className="question-history-placeholder">{message.data?.status === 'preparing' ? '正在生成问题…' : '未收到问题详情。'}</p>}
    {output ? <div className="question-history-result"><span>回答结果</span><p>{output}</p></div> : null}
    {copyText ? <CopyButton text={copyText} label="复制提问详情" /> : null}
  </section>;
}
