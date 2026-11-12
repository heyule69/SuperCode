import { useEffect, useState } from 'react';
import { ArrowLeft, LoaderCircle } from 'lucide-react';
import { call } from './api';
import type { AgentProfile } from './types';

export interface ClaudeMapping {
  mode: 'follow' | 'mapped';
  sonnet: string; opus: string; haiku: string; fable: string; subagent: string;
  forceSubagent: boolean;
}
const roles = ['sonnet', 'opus', 'haiku', 'fable', 'subagent'] as const;
const labels = { sonnet: 'Sonnet', opus: 'Opus', haiku: 'Haiku', fable: 'Fable', subagent: 'Subagent（缺省子智能体）' };

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
    <div className="provider-picker-heading"><h3>Claude 模型映射 · {profile.name}</h3><button type="button" className="quiet-button" disabled={busy} onClick={back}><ArrowLeft size={15}/>返回连接</button></div>
    {error ? <p className="cc-error" role="alert">{error}</p> : !mapping ? <p className="muted"><LoaderCircle size={15} className="spin"/>正在读取模型映射…</p> : <>
      <fieldset disabled={busy || inUse} className="claude-mapping-mode">
        <legend>子智能体模型策略</legend>
        <label><input type="radio" name="claude-model-mode" value="follow" checked={mapping.mode === 'follow'} onChange={() => setMapping({ ...mapping, mode: 'follow' })}/>跟随当前主模型</label>
        <label><input type="radio" name="claude-model-mode" value="mapped" checked={mapping.mode === 'mapped'} onChange={() => setMapping({ ...mapping, mode: 'mapped' })}/>按角色独立映射</label>
      </fieldset>
      <p className="provider-plan-help">{mapping.mode === 'follow' ? '子智能体和 Sonnet、Opus、Haiku、Fable 角色统一使用当前聊天选中的模型；切换主模型时同步更新。' : '填写本连接可调用的真实模型 ID。留空的角色跟随当前主模型；Subagent 用于没有显式指定角色的子智能体。'}</p>
      {mapping.mode === 'mapped' ? <fieldset disabled={busy || inUse} className="claude-mapping-fields">
        <legend>角色对应的实际模型</legend>
        {roles.map(role => <div className="claude-mapping-row" key={role}>
          <label htmlFor={`claude-mapping-${role}`}>{labels[role]}</label>
          <input id={`claude-mapping-${role}`} className="code-input" value={mapping[role]} list={`claude-mapping-models-${role}`} placeholder="跟随当前主模型" autoComplete="off" spellCheck={false} onChange={event => setMapping({ ...mapping, [role]: event.target.value })}/>
          <datalist id={`claude-mapping-models-${role}`}>{models.filter(model => model.toLowerCase().includes(mapping[role].toLowerCase())).slice(0, 100).map(model => <option key={model} value={model}/>)}</datalist>
        </div>)}
        <label className="claude-mapping-force"><input type="checkbox" checked={mapping.forceSubagent} onChange={event => setMapping({ ...mapping, forceSubagent: event.target.checked })}/>固定常规子智能体使用 Subagent 模型</label>
        {mapping.forceSubagent ? <p className="provider-plan-help">固定后，常规子智能体的角色和模型定义由 Subagent 设置覆盖；留空时使用当前主模型。分叉任务和显式继承主模型的技能子任务仍按 Claude 原生规则使用主模型。</p> : null}
      </fieldset> : null}
      <p className="provider-plan-help">固定子智能体模型需要 Claude Code 2.1.257 或更新版本。设置按当前连接保存，从下一次发送生效。</p>
      {inUse ? <p className="provider-runtime-hint" role="status">此连接正被任务使用，完成或停止后可修改模型映射。</p> : null}
      <div className="provider-save-actions"><button className="primary-button" disabled={busy || inUse}>{busy ? <LoaderCircle size={15} className="spin"/> : null}保存模型映射</button></div>
    </>}
  </form>;
}
