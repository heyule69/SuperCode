import { ArrowRightLeft } from 'lucide-react';
import { AgentMenu } from './AgentMenu';
import { LinkageModelPicker } from './LinkageModelPicker';
import type { Agent, AgentProfile } from './types';
import './linkage.css';

export interface LinkageChoice { agentId: string; connectionId: string; modelId: string; effort?: string; serviceTier?: string }
export interface LinkageModel { id: string; name: string }
export interface LinkageConnection { id: string; name: string; models: LinkageModel[] }
export interface LinkageCatalog {
  agent: Agent;
  connections: LinkageConnection[];
  loading?: boolean;
  error?: string;
  live?: boolean;
  profiles?: AgentProfile[];
}
export interface LinkageConfiguration {
  mode: 'single' | 'linked';
  controller: LinkageChoice;
  executor: LinkageChoice;
}

// The host supplies its registered engines and model catalogs. Neither role is
// tied to a specific engine, provider, model, or another role's selection.
export function firstLinkageChoice(catalog: LinkageCatalog): LinkageChoice {
  const connection = catalog.connections[0];
  return { agentId: catalog.agent.id, connectionId: connection?.id ?? '', modelId: connection?.models[0]?.id ?? '' };
}

function RoleCard({ role, value, catalogs, disabled, change, manage }: {
  role: 'controller' | 'executor'; value: LinkageChoice; catalogs: LinkageCatalog[];
  disabled: boolean; change: (value: LinkageChoice) => void; manage: (agentId: string) => void;
}) {
  const name = role === 'controller' ? '主控' : '执行';
  const catalog = catalogs.find(item => item.agent.id === value.agentId);
  return <section className="linkage-role-row" aria-label={`${name} Agent 配置`}>
    <strong className="linkage-role-label">{name}</strong>
    <AgentMenu key={value.agentId} label={`选择${name} Agent`} value={value.agentId} agents={catalogs.map(item => item.agent)} busy={disabled}
      choose={id => { const next = catalogs.find(item => item.agent.id === id); if (next) change(firstLinkageChoice(next)); }}/>
    <div className="linkage-role-model"><LinkageModelPicker value={value} catalog={catalog} disabled={disabled} onChange={change} manage={manage}/></div>
  </section>;
}

export function LinkageSetup({ value, catalogs, onChange, manage, disabled = false }: {
  value: LinkageConfiguration; catalogs: LinkageCatalog[]; onChange: (value: LinkageConfiguration) => void;
  manage: (agentId: string) => void; disabled?: boolean;
}) {
  return <section className="linkage-setup" aria-label="工作方式配置">
    <div className="linkage-enable-row"><span>启用联动</span>
      <button type="button" className="linkage-toggle" role="switch" aria-label="启用联动" aria-checked={value.mode === 'linked'} disabled={disabled}
        onClick={() => onChange({ ...value, mode: value.mode === 'linked' ? 'single' : 'linked' })}><span/></button>
    </div>
    <div className={`linkage-roles${value.mode === 'single' ? ' linkage-single' : ''}`}>
      <RoleCard role="controller" value={value.controller} catalogs={catalogs} disabled={disabled} manage={manage}
        change={controller => onChange({ ...value, controller })}/>
      {value.mode === 'linked' ? <>
        <button type="button" className="linkage-swap" aria-label="交换主控和执行 Agent" title="交换主控和执行 Agent" disabled={disabled}
          onClick={() => onChange({ ...value, controller: value.executor, executor: value.controller })}><ArrowRightLeft size={15}/></button>
        <RoleCard role="executor" value={value.executor} catalogs={catalogs} disabled={disabled} manage={manage}
          change={executor => onChange({ ...value, executor })}/>
      </> : null}
    </div>
  </section>;
}
