import { useState } from 'react';
import { ShieldCheck, MessageCircleQuestion } from 'lucide-react';
import { initialAnswers } from './events';
import { fieldOptions, formContent, formDefaults, supportedForm, type ElicitationField } from './elicitation';
import type { RpcEvent } from './types';
import { CopyButton } from './CopyButton';

export default function RequestCard({ request, respond }: { request: RpcEvent; respond: (result: unknown) => Promise<void> }) {
  const [answers, setAnswers] = useState(() => initialAnswers(request));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [choices, setChoices] = useState<Record<string, string[]>>({});
  const p = request.params;
  const schema = p.requestedSchema ?? {};
  const [form, setForm] = useState<Record<string, unknown>>(() => formDefaults(schema));
  const formMode = ['form', 'openai/form', 'openaiForm'].includes(p.mode);
  const canFill = formMode && supportedForm(schema);
  const fields = Object.entries(schema.properties ?? {}) as [string, ElicitationField][];
  const question = request.method === 'item/tool/requestUserInput';
  const permissions = request.method === 'item/permissions/requestApproval';
  const elicitation = request.method === 'mcpServer/elicitation/request';
  const approval = ['item/commandExecution/requestApproval', 'item/fileChange/requestApproval', 'claude/tool/requestApproval'].includes(request.method);
  const sessionApproval = request.method === 'item/commandExecution/requestApproval' && (!p.availableDecisions || p.availableDecisions.includes('acceptForSession'));
  async function submit(accept: boolean, forSession = false) {
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      const result = question ? { answers: Object.fromEntries(Object.entries(answers).map(([id, value]) => [id, { answers: [...(choices[id] ?? []), ...(value.trim() ? [value.trim()] : [])] }])) }
        : permissions ? { permissions: accept ? p.permissions ?? {} : {}, scope: 'turn' }
        : elicitation ? { action: accept ? 'accept' : 'decline', content: accept && canFill ? formContent(schema, form) : null }
        : approval ? { decision: accept ? forSession && sessionApproval ? 'acceptForSession' : 'accept' : 'decline' }
        : null;
      await respond(result);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); } finally { setBusy(false); }
  }
  return <section className="request-card" aria-busy={busy} aria-label={question ? 'Agent 提问' : 'Agent 审批请求'}>
    <div className="request-title">{question ? <MessageCircleQuestion size={17} /> : <ShieldCheck size={17} />}<strong>{question ? 'Agent 需要你的回答' : elicitation ? `${p.serverName ?? 'MCP'} 请求${p.mode === 'url' ? '完成授权' : '填写信息'}` : p.toolName ? `请求使用 ${p.toolName}` : 'Agent 正在等待确认'}</strong><span className="approval-wait">等待你确认</span></div>
    {question ? (p.questions ?? []).map((q: any) => <fieldset className="question" key={q.id}><legend>{q.header ? <strong>{q.header} · </strong> : null}{q.question}{q.multiSelect ? <small>（可多选）</small> : null}</legend><div className="question-options">{(q.options ?? []).map((o: any) => <button type="button" key={o.label} disabled={busy} aria-pressed={(choices[q.id] ?? []).includes(o.label)} className={(choices[q.id] ?? []).includes(o.label) ? 'selected' : ''} onClick={() => { setChoices(old => { const selected = old[q.id] ?? []; return { ...old, [q.id]: q.multiSelect ? selected.includes(o.label) ? selected.filter(v => v !== o.label) : [...selected, o.label] : selected.includes(o.label) ? [] : [o.label] }; }); if (!q.multiSelect) setAnswers(a => ({ ...a, [q.id]: '' })); }}><strong>{o.label}</strong>{o.description ? <small>{o.description}</small> : null}</button>)}</div><label className="sr-only" htmlFor={`answer-${q.id}`}>自定义回答：{q.question}</label><input id={`answer-${q.id}`} disabled={busy} value={answers[q.id] ?? ''} onChange={e => { setAnswers(a => ({ ...a, [q.id]: e.target.value })); if (!q.multiSelect) setChoices(old => ({ ...old, [q.id]: [] })); }} placeholder="也可以输入自己的回答" /></fieldset>)
      : elicitation ? <><p>{p.message}</p>{canFill ? <div className="elicitation-fields">{fields.map(([name, field]) => <label key={name}><span>{field.title ?? name}{schema.required?.includes(name) ? ' *' : ''}</span>{field.description ? <small>{field.description}</small> : null}{field.type === 'boolean' ? <input type="checkbox" checked={form[name] === true} disabled={busy} onChange={e => setForm(v => ({...v, [name]:e.target.checked}))} /> : field.type === 'array' ? <div className="question-options">{fieldOptions(field.items ?? {}).map(o => <button key={o.value} type="button" aria-pressed={(form[name] as string[] ?? []).includes(o.value)} disabled={busy} onClick={() => setForm(v => { const selected=v[name] as string[] ?? []; return {...v,[name]:selected.includes(o.value)?selected.filter(a=>a!==o.value):[...selected,o.value]}; })}>{o.label}</button>)}</div> : fieldOptions(field).length ? <select value={String(form[name] ?? '')} disabled={busy} onChange={e => setForm(v=>({...v,[name]:e.target.value}))}><option value="">请选择</option>{fieldOptions(field).map(o=><option key={o.value} value={o.value}>{o.label}</option>)}</select> : <input type={field.type==='integer'||field.type==='number'?'number':field.format==='password'?'password':'text'} min={field.minimum} max={field.maximum} step={field.type==='integer'?1:'any'} value={String(form[name]??'')} disabled={busy} onChange={e=>setForm(v=>({...v,[name]:e.target.value}))} />}</label>)}</div> : p.mode === 'url' ? <div className="elicitation-url"><code>{p.url}</code><CopyButton className="quiet-button" text={p.url} label="复制授权链接" /><small>在浏览器中完成后，再点击“我已完成”。</small></div> : <p>此表单包含暂不支持的字段，可拒绝后继续。</p>}</>
      : <><p>{p.reason ?? p.message ?? (permissions ? '请求额外的文件或网络权限' : approval ? '请检查本次操作，再决定是否允许。' : '此类型请求暂不能在界面中填写，可以拒绝后继续。')}</p>{p.cwd ? <div className="request-cwd">工作目录 <code>{p.cwd}</code></div> : null}<pre>{p.command ?? p.grantRoot ?? JSON.stringify(p.permissions ?? p.input ?? p.changes ?? p, null, 2)}</pre><p className="field-hint">允许本次仅批准当前操作；拒绝后 Agent 可调整方案。审批规则可以在任务结束后切换。</p></>}
    {error ? <p className="request-error" role="alert">{error}</p> : null}
    <div className="request-actions">{question ? <button className="primary-button" disabled={busy || Object.entries(answers).some(([id, a]) => !a.trim() && !choices[id]?.length)} onClick={() => void submit(true)}>{busy ? '正在提交…' : '提交回答'}</button> : <><button className="quiet-button" disabled={busy} onClick={() => void submit(false)}>拒绝</button>{sessionApproval ? <button className="quiet-button" disabled={busy} title="使用 Codex 原生会话审批缓存，在本会话中允许匹配的命令" onClick={() => void submit(true, true)}>本会话允许此命令</button> : null}{elicitation && (canFill || p.mode === 'url') ? <button className="primary-button" disabled={busy} onClick={() => void submit(true)}>{busy ? '正在提交…' : p.mode === 'url' ? '我已完成' : '提交'}</button> : null}{approval || permissions ? <button className="primary-button" disabled={busy} onClick={() => void submit(true)}>{busy ? '正在提交…' : '允许本次'}</button> : null}</>}</div>
  </section>;
}
