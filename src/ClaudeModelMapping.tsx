import { useEffect, useState } from 'react';
import { ArrowLeft, Info, LoaderCircle } from 'lucide-react';
import { call } from './api';
import type { AgentProfile } from './types';

export interface ClaudeMapping {
  mode: 'follow' | 'mapped';
  sonnet: string; opus: string; haiku: string; fable: string; subagent: string;
  forceSubagent: boolean;
}
const roles = ['sonnet', 'opus', 'haiku', 'fable', 'subagent'] as const;
const labels = { sonnet: 'Sonnet', opus: 'Opus', haiku: 'Haiku', fable: 'Fable', subagent: 'Subagent' };

export function ClaudeModelMapping({ profile, busy, inUse, back, save }: {
  profile: AgentProfile; busy: boolean; inUse: boolean; back: () => void; save: (mapping: ClaudeMapping) => void;
}) {
  const [mapping, setMapping] = useState<ClaudeMapping>();
  const [error, setError] = useState('');
  useEffect(() => {
    let disposed = false;
    setMapping(undefined); setError('');
    void call<ClaudeMapping>('get_claude_model_mapping', { id: profile.id }).then(value => {
      if (!disposed) setMapping(value);
    }).catch(error => { if (!disposed) setError(error instanceof Error ? error.message : String(error)); });
    return () => { disposed = true; };
  }, [profile.id]);
  const models = [...new Set([profile.model, ...profile.models ?? []].filter((model): model is string => !!model))];
  return <form className="provider-editor claude-model-mapping" aria-label="Claude 模型映射" onSubmit={event => { event.preventDefault(); if (mapping && !busy && !inUse) save(mapping); }}>
    <header className="claude-mapping-header"><div><h3>Claude 模型映射</h3><span className="claude-mapping-connection">{profile.name}</span></div><button type="button" className="quiet-button" disabled={busy} onClick={back}><ArrowLeft size={15}/>返回连接</button></header>
    <div className="claude-mapping-body">
    {error ? <p className="cc-error" role="alert">{error}</p> : !mapping ? <p className="claude-mapping-loading"><LoaderCircle size={15} className="spin"/>正在读取模型映射…</p> : <>
      <fieldset disabled={busy || inUse} className="claude-mapping-mode">
        <legend className="sr-only">子智能体模型策略</legend>
        <div className="claude-mapping-mode-line"><span className="claude-mapping-mode-label" aria-hidden="true">模型策略</span>
        <div className="claude-mapping-choices">
          <label className={`claude-mapping-choice${mapping.mode === 'follow' ? ' is-selected' : ''}`}><input type="radio" name="claude-model-mode" value="follow" aria-label="跟随当前主模型" checked={mapping.mode === 'follow'} onChange={() => setMapping({ ...mapping, mode: 'follow' })}/><span className="claude-mapping-choice-title"><strong>跟随主模型</strong></span></label>
          <label className={`claude-mapping-choice${mapping.mode === 'mapped' ? ' is-selected' : ''}`}><input type="radio" name="claude-model-mode" value="mapped" aria-label="按角色独立映射" checked={mapping.mode === 'mapped'} onChange={() => setMapping({ ...mapping, mode: 'mapped' })}/><span className="claude-mapping-choice-title"><strong>独立映射</strong></span></label>
        </div></div>
      </fieldset>
      {mapping.mode === 'mapped' ? <fieldset disabled={busy || inUse} className="claude-mapping-fields">
        <legend className="sr-only">角色对应的实际模型</legend>
        <div className="claude-mapping-columns" aria-hidden="true"><span>角色</span><span>调用模型<small>留空跟随主模型</small></span></div>
        {roles.map(role => <div className="claude-mapping-row" key={role}>
          <label htmlFor={`claude-mapping-${role}`} title={role === 'subagent' ? '未指定角色时使用的子智能体模型' : undefined}>{labels[role]}</label>
          <input id={`claude-mapping-${role}`} className="code-input" value={mapping[role]} list={`claude-mapping-models-${role}`} placeholder="跟随当前主模型" autoComplete="off" spellCheck={false} onChange={event => setMapping({ ...mapping, [role]: event.target.value })}/>
          <datalist id={`claude-mapping-models-${role}`}>{models.filter(model => model.toLowerCase().includes(mapping[role].toLowerCase())).slice(0, 100).map(model => <option key={model} value={model}/>)}</datalist>
        </div>)}
        <div className="claude-mapping-force-section"><label className="claude-mapping-force" title="固定后，常规子智能体统一使用 Subagent 模型；留空时跟随主模型。分叉及显式继承主模型的技能子任务仍使用主模型。"><span>固定子智能体</span><input type="checkbox" role="switch" aria-label="固定子智能体模型" checked={mapping.forceSubagent} onChange={event => setMapping({ ...mapping, forceSubagent: event.target.checked })}/></label></div>
      </fieldset> : null}
      {inUse ? <p className="provider-runtime-hint" role="status">此连接正被任务使用，暂不可修改。</p> : null}
    </>}
    </div>
    {mapping && !error ? <footer className="claude-mapping-footer"><span title="固定子智能体模型需要 Claude Code 2.1.257 或更新版本。">下次发送生效<Info size={13} aria-label="版本要求"/></span><button className="primary-button" aria-label="保存模型映射" disabled={busy || inUse}>{busy ? <LoaderCircle size={15} className="spin"/> : null}保存</button></footer> : null}
  </form>;
}
