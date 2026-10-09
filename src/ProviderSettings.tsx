import { lazy, Suspense, useEffect, useRef, useState, type PointerEvent, type SelectHTMLAttributes } from 'react';
import { ArrowLeft, ArrowUp, ArrowDown, Check, ChevronDown, ChevronRight, Download, ExternalLink, GripVertical, LoaderCircle, Plus, RefreshCw, Search, Trash2, X } from 'lucide-react';
import catalog from '../resources/providers.json';
import { CredentialInput } from './CredentialInput';
import { AgentIcon, ProviderIcon } from './AgentIcon';
import { call, desktop } from './api';
import type { AgentProfile, Provider, ProviderSettings as Config } from './types';
import { connectionIds, mergeVisibleOrder, moveConnection, nativeConnectionSource, providerAgents, visibleProviderIds, type ConnectionOrder } from './providerOrder';
const providers = catalog as Provider[];
const CCSwitchSettings = lazy(() => import('./CCSwitchSettings'));
const protocolLabels: Record<string, string> = { anthropic: 'Anthropic Messages', chat: 'OpenAI Chat Completions', responses: 'OpenAI Responses' };
const blank: Config = { id: null, name: '', agent: 'claude', providerId: 'custom', plan: 'anthropic', protocol: 'anthropic', baseUrl: '', model: '', models: [], hasCredential: false };
type AccountStatus = { agent: string; loggedIn: boolean; method?: string; plan?: string; error?: string | null };

function SelectControl({ children, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return <div className="select-control"><select {...props}>{children}</select><ChevronDown size={14} aria-hidden="true" /></div>;
}

export default function ProviderSettings({ profiles, officialAgents, order, busy, updated }: { profiles: AgentProfile[]; officialAgents: string[]; order?: ConnectionOrder; busy: boolean; updated: (resetAgent?: string) => Promise<void> }) {
  const [adding, setAdding] = useState(false);
  const [apiConnections, setApiConnections] = useState(false);
  const [importing, setImporting] = useState(false);
  const [agentTab, setAgentTab] = useState('claude');
  const [nativeId, setNativeId] = useState<string | null>(null);
  const [dragId, setDragId] = useState('');
  const [dropTarget, setDropTarget] = useState<{ id: string; edge: 'before' | 'after' } | null>(null);
  const providerList = useRef<HTMLDivElement>(null);
  const pointerDrag = useRef<{ id: string; y: number; pointerId: number; active: boolean; target?: { id: string; edge: 'before' | 'after' } }>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [search, setSearch] = useState('');
  const [modelSearch, setModelSearch] = useState('');
  const [connectionSearch, setConnectionSearch] = useState('');
  const [newModel, setNewModel] = useState('');
  const [working, setWorking] = useState(false);
  const [credentialRevision, setCredentialRevision] = useState(0);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [deleteId, setDeleteId] = useState('');
  const [account, setAccount] = useState<Record<string, { loggedIn: boolean; method?: string; plan?: string }>>({});
  const [accountsChecking, setAccountsChecking] = useState(desktop);
  const [login, setLogin] = useState<{ agent: string; id?: string } | null>(() => { try { return JSON.parse(sessionStorage.getItem('supercode.pendingLogin') ?? 'null'); } catch { return null; } });
  useEffect(() => { if (login) sessionStorage.setItem('supercode.pendingLogin', JSON.stringify(login)); else sessionStorage.removeItem('supercode.pendingLogin'); }, [login]);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    void call<AccountStatus[]>('list_provider_accounts', { force: false }).then(rows => {
      if (!disposed) { setAccount(previous => ({ ...Object.fromEntries(rows.map(row => [row.agent, row])), ...previous })); const errors = rows.filter(row => row.error).map(row => row.error); if (errors.length) setError(errors.join('；')); }
    }).catch(e => { if (!disposed) setError(String(e)); }).finally(() => { if (!disposed) setAccountsChecking(false); });
    return () => { disposed = true; };
  }, []);
  async function refreshAccounts() {
    setAccountsChecking(true);
    try {
      const rows = await call<AccountStatus[]>('list_provider_accounts', { force: true });
      setAccount(Object.fromEntries(rows.map(row => [row.agent, row])));
      const failed = rows.filter(row => row.error);
      if (failed.length) throw new Error(failed.map(row => row.error).join('；'));
    } finally { setAccountsChecking(false); }
  }
  const active = config ? profiles.find(p => p.id === config.id)?.current : false;
  const provider = providers.find(p => p.id === config?.providerId);
  const completeIds = connectionIds(agentTab, profiles, officialAgents, order);
  const loggedInAgents = Object.keys(account).filter(agent => account[agent].loggedIn);
  const ids = visibleProviderIds(agentTab, profiles, loggedInAgents, order);
  const visibleConnections = ids.map((id, index) => {
    const p = profiles.find(p => p.agent === agentTab && p.id === id);
    const native = nativeConnectionSource(agentTab, id);
    const name = p?.name ?? native.connectionName!;
    const subtitle = p ? `${p.model ?? '沿用 CLI 模型'}${p.source === 'ccswitch' ? ' · CC Switch 导入' : ''}` : id === '@official' ? '使用本机官方登录' : '沿用本机模型与供应商';
    return { id, index, p, native, name, subtitle, isDefault: completeIds[0] === id };
  }).filter(c => `${c.name} ${c.subtitle} ${c.p?.modelSource?.providerName ?? ''}`.toLowerCase().includes(connectionSearch.trim().toLowerCase()));
  const sortingDisabled = !desktop || working || !!connectionSearch.trim();
  const nativeAccountPicker = ['codex', 'claude'].includes(agentTab) && !apiConnections;
  const pickerProviders = providers.map(p => {
    const officialAgent = nativeAccountPicker ? p.id === 'openai' ? 'codex' : p.id === 'anthropic' ? 'claude' : null : null;
    const name = officialAgent ? nativeConnectionSource(officialAgent, '@official').connectionName! : p.id === 'openai' ? 'OpenAI API' : p.id === 'anthropic' ? 'Anthropic API' : p.name;
    const subtitle = officialAgent ? account[officialAgent]?.loggedIn ? '已登录 · 使用本机账号' : '浏览器登录 · 无需 API Key' : `${p.category} · ${p.presets.some(v => /coding|token/.test(v.id)) ? 'Coding Plan / API' : p.category === '自定义' ? '兼容 API' : '标准 API'}`;
    return { p, officialAgent, name, subtitle };
  }).filter(({ p, name, subtitle }) => `${p.name} ${name} ${subtitle}`.toLowerCase().includes(search.trim().toLowerCase()));
  function clearDrag() { pointerDrag.current = null; setDragId(''); setDropTarget(null); }
  function dragMove(e: PointerEvent<HTMLButtonElement>) {
    const drag = pointerDrag.current; const area = providerList.current;
    if (!drag || drag.pointerId !== e.pointerId || !area || sortingDisabled) return;
    if (!drag.active && Math.abs(e.clientY - drag.y) < 4) return;
    e.preventDefault(); drag.active = true; setDragId(drag.id);
    const bounds = area.getBoundingClientRect();
    if (e.clientY < bounds.top + 24) area.scrollTop -= 16;
    else if (e.clientY > bounds.bottom - 24) area.scrollTop += 16;
    const rows = Array.from(area.querySelectorAll<HTMLElement>('.provider-sort-row'));
    const target = rows.find(row => { const rect = row.getBoundingClientRect(); return e.clientY >= rect.top && e.clientY < rect.bottom; })
      ?? (e.clientY < bounds.top ? rows[0] : e.clientY >= bounds.bottom ? rows[rows.length - 1] : undefined);
    const id = target?.dataset.connectionId;
    if (!target || !id || id === drag.id) { drag.target = undefined; setDropTarget(null); return; }
    const rect = target.getBoundingClientRect();
    drag.target = { id, edge: e.clientY < rect.top + rect.height / 2 ? 'before' : 'after' };
    setDropTarget(drag.target);
  }
  function dragEnd(e: PointerEvent<HTMLButtonElement>) {
    const drag = pointerDrag.current;
    if (!drag || drag.pointerId !== e.pointerId) return;
    const next = drag.active && drag.target ? moveConnection(ids, drag.id, drag.target.id, drag.target.edge) : ids;
    clearDrag(); e.currentTarget.releasePointerCapture(e.pointerId);
    void reorder(next);
  }
  async function reorder(next: string[]) {
    if (sortingDisabled || next.every((id, i) => id === ids[i]) && next[0] === completeIds[0]) return;
    await run(async () => {
      await call('reorder_provider_connections', { agent: agentTab, ids: mergeVisibleOrder(next, completeIds) });
      await updated(); setMessage('顺序已保存 · 第一项为新聊天默认');
    });
  }
  function move(id: string, direction: -1 | 1) {
    const index = ids.indexOf(id); const target = ids[index + direction];
    if (target) void reorder(moveConnection(ids, id, target, direction === -1 ? 'before' : 'after'));
  }
  function accountControls(agent: string) {
    return <div className="provider-account-controls"><p className="muted">{account[agent]?.loggedIn ? `已登录 · ${account[agent].plan ?? account[agent].method ?? '官方账号'}` : account[agent] ? '本机官方账号未登录' : '使用本机官方登录，凭据由 CLI 管理。'}</p><div className="account-actions">
      <button className="quiet-button" disabled={!desktop || working || busy} onClick={() => void run(async () => { const result = await call<{ loggedIn: boolean }>('official_account_status', { agent }); setAccount(a => ({ ...a, [agent]: result })); setMessage(result.loggedIn ? '本机账号已登录' : '本机账号未登录'); })}>检查登录</button>
      <button className="quiet-button" disabled={!desktop || working || busy || !!login} onClick={() => void run(async () => { const result = await call<{ loginId?: string; message: string }>('start_official_login', { agent }); setLogin({ agent, id: result.loginId }); await updated(); setMessage(result.message); })}><ExternalLink size={13} />登录</button>
    </div></div>;
  }
  async function run(action: () => Promise<void>) {
    setWorking(true); setMessage(''); setError('');
    try { await action(); } catch (e) { setError(String(e)); } finally { setWorking(false); }
  }
  function choose(p: Provider, presetId = p.presets[0].id) {
    const preset = p.presets.find(p => p.id === presetId)!;
    setConfig({ ...blank, providerId: p.id, plan: preset.id, name: p.id === 'custom' ? '自定义连接' : `${p.name} · ${preset.name}`, protocol: preset.protocol, agent: ['opencode', 'pi'].includes(agentTab) ? agentTab : preset.protocol === 'responses' ? 'codex' : 'claude', baseUrl: preset.baseUrl, model: preset.models[0] ?? '', models: preset.models });
    setAdding(false); setNativeId(null); setModelSearch(''); setNewModel(''); setError(''); setMessage('');
  }
  async function chooseOfficial(agent: string) {
    setAgentTab(agent); setConfig(null); setNativeId(null);
    const status = account[agent] ?? await call<AccountStatus>('official_account_status', { agent });
    setAccount(previous => ({ ...previous, [agent]: status }));
    if (status.loggedIn) {
      await call('use_official_account', { agent });
      await updated(agent); setAdding(false); setNativeId('@official'); setMessage('已使用本机官方账号');
    } else {
      const result = await call<{ loginId?: string; message: string }>('start_official_login', { agent });
      setLogin({ agent, id: result.loginId }); setMessage(result.message);
    }
  }
  async function save(activate = false) {
    if (!config) return;
    const saved = await call<AgentProfile>('save_provider_profile', { settings: { ...config, apiKey: config.apiKey?.trim() || null }, activate });
    setConfig(c => c ? { ...c, id: saved.id, apiKey: '', hasCredential: saved.hasCredential } : null);
    setCredentialRevision(revision => revision + 1);
    await updated(activate || saved.current ? saved.agent : undefined); setMessage(activate ? '已保存并启用' : '已保存');
    return saved.id;
  }
  return <div className="provider-settings">
    <div className="settings-page-heading"><h2>模型供应商</h2><div>{!desktop ? <span className="preview-label" title="预览模式不保存密钥，不调用模型">预览</span> : null}<button className="quiet-button" disabled={!desktop || working || accountsChecking} onClick={() => void run(async () => { await refreshAccounts(); await updated(); })}><RefreshCw size={15} className={accountsChecking ? 'spin' : ''}/>{accountsChecking ? '检查中…' : '刷新连接'}</button><button className="quiet-button" disabled={working || busy} onClick={() => { setImporting(true); setConfig(null); setNativeId(null); setAdding(false); setError(''); setMessage(''); }}><Download size={15} />CC Switch 导入</button><button className="primary-button" disabled={working || busy} onClick={() => { setAdding(true); setApiConnections(false); setImporting(false); setConfig(null); setNativeId(null); setSearch(''); setError(''); setMessage(''); }}><Plus size={15} />添加</button></div></div>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    {message ? <p className="provider-feedback" role="status"><Check size={15} />{message}</p> : null}
    <div className="provider-view">{importing ? <Suspense fallback={<p className="muted">加载中…</p>}><CCSwitchSettings busy={busy || working} updated={updated} back={() => setImporting(false)} /></Suspense> : adding ? <div className="provider-picker">
      <div className="provider-picker-heading"><h3>添加连接</h3>{['codex', 'claude'].includes(agentTab) ? <button className="quiet-button" disabled={working || busy} onClick={() => setApiConnections(value => !value)}>{apiConnections ? '使用官方账号' : '使用 API Key'}</button> : null}<button className="quiet-button" disabled={working} onClick={() => setAdding(false)}><ArrowLeft size={15} />返回</button></div>
      <div className="search-field"><Search size={16} /><input autoFocus value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索供应商" aria-label="搜索供应商" /></div>
      <div className="provider-preset-grid">{pickerProviders.map(({ p, officialAgent, name, subtitle }) => <button key={p.id} className="provider-preset" disabled={working || busy || !!officialAgent && (!desktop || !!login)} onClick={() => officialAgent ? void run(() => chooseOfficial(officialAgent)) : choose(p)}><span className={`provider-mark mark-${p.id}`}><ProviderIcon provider={p.id} name={name} mark={p.mark}/></span><span><strong>{name}</strong><small>{subtitle}</small></span>{officialAgent ? <ExternalLink size={15} /> : <ChevronRight size={15} />}</button>)}</div>
      {!pickerProviders.length ? <p className="muted">无匹配供应商</p> : null}
    </div> : nativeId ? <div className="provider-native-editor">
      <button className="quiet-button" disabled={working} onClick={() => setNativeId(null)}><ArrowLeft size={15} />{providerAgents.find(a => a.id === agentTab)?.name}</button>
      <div className="provider-native-heading"><span className="provider-mark">{nativeId === '@official' ? <ProviderIcon provider={agentTab === 'claude' ? 'anthropic' : 'openai'}/> : <AgentIcon agent={agentTab}/>}</span><h3>{nativeConnectionSource(agentTab, nativeId).connectionName}</h3>{completeIds[0] === nativeId ? <span className="connection-state enabled">新聊天默认</span> : null}</div>
      {nativeId === '@official' ? accountControls(agentTab) : <p className="muted">沿用本机 {providerAgents.find(a => a.id === agentTab)?.name} 的模型与供应商配置。</p>}
      <button className="quiet-button" disabled={!desktop || working || busy || completeIds[0] === nativeId} onClick={() => void reorder(moveConnection(ids, nativeId, ids[0]))}>移到第一位，设为默认</button>
    </div> : config ? <div className="provider-editor">
      <div className="provider-editor-heading"><button className="quiet-button" disabled={working} onClick={() => { setConfig(null); setDeleteId(''); }}><ArrowLeft size={15} />所有连接</button><span className={`connection-state ${active ? 'enabled' : ''}`}>{active ? '新会话默认' : config.id ? '已保存' : '新连接'}</span></div>
      <div className="provider-config-grid"><div className="provider-config-fields">
        <h3>连接</h3>
        <label htmlFor="provider-name">连接名称</label><input id="provider-name" value={config.name} disabled={working || busy} onChange={e => setConfig({ ...config, name: e.target.value })} autoComplete="off" />
        {config.official ? accountControls(config.agent) : <>
        {config.providerId !== 'custom' ? <><label htmlFor="provider-plan" title="切换套餐将新建连接，需要对应的 API Key">套餐 / 接入方式</label><SelectControl id="provider-plan" value={config.plan} disabled={working || busy} onChange={e => { if (provider) { const p = provider.presets.find(p => p.id === e.target.value)!; setConfig({ ...config, name: provider.presets.some(v => config.name === `${provider.name} · ${v.name}`) ? `${provider.name} · ${p.name}` : config.name, plan: p.id, protocol: p.protocol, baseUrl: p.baseUrl, agent: ['opencode', 'pi'].includes(config.agent) ? config.agent : p.protocol === 'responses' ? 'codex' : 'claude', model: p.models[0] ?? '', models: p.models, apiKey: '', hasCredential: false, id: null }); } }}>{provider?.presets.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}{!provider?.presets.some(p => p.id === config.plan) ? <option value={config.plan}>导入的配置</option> : null}</SelectControl></> : null}
        <label htmlFor="provider-protocol">API 协议</label><SelectControl id="provider-protocol" value={config.protocol} disabled={working || busy || !!config.id} onChange={e => setConfig({ ...config, protocol: e.target.value, plan: config.providerId === 'custom' ? e.target.value : config.plan, agent: ['opencode', 'pi'].includes(config.agent) ? config.agent : e.target.value === 'responses' ? 'codex' : 'claude' })}>{Object.entries(protocolLabels).map(([id, name]) => <option key={id} value={id}>{name}</option>)}</SelectControl>
        <label htmlFor="provider-url">Base URL</label><input id="provider-url" className="code-input" value={config.baseUrl} disabled={working || busy} onChange={e => setConfig({ ...config, baseUrl: e.target.value })} placeholder="https://api.example.com/v1" spellCheck={false} autoComplete="off" />
        <label htmlFor="provider-key" title="Windows 使用本机用户加密保存密钥">API Key {config.hasCredential ? <span className="credential-saved"><Check size={12} />已保存</span> : null}</label><CredentialInput key={config.id ?? 'new'} id="provider-key" saved={config.hasCredential} revision={credentialRevision} value={config.apiKey ?? ''} disabled={working || busy} onChange={apiKey => setConfig({ ...config, apiKey })} />
        </>}
        <div className="provider-engine-note"><span>引擎</span><strong>{providerAgents.find(a => a.id === config.agent)?.name}</strong></div>
      </div><div className="provider-model-manager">
        <div className="settings-section-heading"><h3>模型</h3><span className="model-count">{config.models.length} 个</span></div>
        <label htmlFor="provider-default-model">默认模型 ID</label><input id="provider-default-model" className="code-input" value={config.model} disabled={working || busy} onChange={e => setConfig({ ...config, model: e.target.value })} placeholder="模型 ID" spellCheck={false} autoComplete="off" />
        {config.models.length ? <div className="search-field model-search"><Search size={15} /><input aria-label="搜索已添加模型" placeholder="搜索模型" value={modelSearch} onChange={e => setModelSearch(e.target.value)} /></div> : null}
        <div className={`provider-model-list ${!config.models.length ? 'is-empty' : ''}`}>{config.models.filter(m => m.toLowerCase().includes(modelSearch.toLowerCase())).map(m => <div className="provider-model-row" key={m}><button disabled={working || busy} aria-pressed={config.model === m} aria-label={`默认模型 ${m}`} onClick={() => setConfig({ ...config, model: m })}><span className={`model-radio ${config.model === m ? 'selected' : ''}`} /><span title={m}>{m}</span>{config.model === m ? <small>默认</small> : null}</button><button className="icon-button" title={`移除模型 ${m}`} disabled={working || busy || m === config.model} onClick={() => setConfig({ ...config, models: config.models.filter(v => v !== m) })}><X size={14} /></button></div>)}{!config.models.length ? <p className="muted">暂无模型</p> : !config.models.some(m => m.toLowerCase().includes(modelSearch.toLowerCase())) ? <p className="muted">没有匹配的模型</p> : null}</div>
        <form className="add-model-row" onSubmit={e => { e.preventDefault(); const m = newModel.trim(); if (m && !config.models.includes(m)) setConfig({ ...config, models: [...config.models, m], model: config.model || m }); setNewModel(''); }}><input aria-label="新增模型 ID" className="code-input" value={newModel} onChange={e => setNewModel(e.target.value)} placeholder="添加模型 ID" spellCheck={false} autoComplete="off" disabled={working || busy} /><button className="quiet-button" disabled={!newModel.trim() || working || busy}><Plus size={15} />添加</button></form>
        <button className="quiet-button" disabled={!desktop || working || busy || (!config.official && !config.baseUrl.trim()) || !config.model.trim()} title="先保存连接，再获取模型列表" onClick={() => void run(async () => { const id = await save(); if (!id) return; const models = await call<string[]>('fetch_provider_models', { id }); setConfig(c => c ? { ...c, models: [...new Set([c.model, ...models])].slice(0, 200) } : null); setMessage(`已获取 ${models.length} 个模型，保存后生效`); })}><RefreshCw size={14} />获取模型</button>
      </div></div>
      <div className="provider-editor-footer">
      {config.id ? <div className="provider-delete">{deleteId === config.id ? <><span>删除连接？</span><button className="quiet-button" onClick={() => setDeleteId('')}>取消</button><button className="danger-button" disabled={working || busy} onClick={() => void run(async () => { await call('delete_provider_profile', { id: config.id }); setConfig(null); setDeleteId(''); await updated(); setMessage('已删除'); })}>删除</button></> : <button className="quiet-button" disabled={working || busy} onClick={() => setDeleteId(config.id!)}><Trash2 size={14} />删除</button>}</div> : <span />}
      <div className="provider-save-actions">{working ? <LoaderCircle size={15} className="spin" /> : null}<button className="quiet-button" disabled={!desktop || working || busy || !config.model.trim() || (!config.official && !config.baseUrl.trim())} title="先保存，再发送测试请求；按供应商规则计费" onClick={() => void run(async () => { const id = await save(); if (!id) return; const result = await call<{ model: string }>('test_provider_connection', { id }); setMessage(config.official ? '官方账号可用' : `连接成功 · ${result.model}`); })}>{config.official ? '检查账号' : '测试'}</button><button className="quiet-button" disabled={!desktop || working || busy || !config.model.trim()} onClick={() => void run(async () => { await save(); })}>保存</button><button className="primary-button" disabled={!desktop || working || busy || !config.model.trim()} onClick={() => void run(async () => { await save(true); })}>保存为默认</button></div></div>
    </div> : <div className="provider-overview">
      <div className="provider-tabs" role="tablist" aria-label="按 Agent 分类">{providerAgents.map(a => <button key={a.id} role="tab" id={`providers-${a.id}-tab`} aria-controls="provider-connections" aria-selected={agentTab === a.id} className={agentTab === a.id ? 'selected' : ''} disabled={working} onClick={() => { setAgentTab(a.id); setConnectionSearch(''); setMessage(''); clearDrag(); }}><AgentIcon agent={a.id}/>{a.name}<span>{visibleProviderIds(a.id, profiles, loggedInAgents, order).length}</span></button>)}</div>
      <div className="search-field"><Search size={15} /><input aria-label="搜索模型连接" placeholder="搜索连接或模型" value={connectionSearch} onChange={e => { setConnectionSearch(e.target.value); clearDrag(); }} /></div>
      <p className="provider-order-hint">{connectionSearch.trim() ? '清空搜索后可调整顺序' : !ids.length ? accountsChecking && ['codex', 'claude'].includes(agentTab) ? '正在检查本机官方登录…' : '添加连接或从 CC Switch 导入' : `拖动调整顺序并设置 ${providerAgents.find(a => a.id === agentTab)?.name} 新聊天默认`}</p>
      <div ref={providerList} className="provider-scroll" id="provider-connections" role="tabpanel" aria-labelledby={`providers-${agentTab}-tab`}>
        <div className="saved-provider-list">{visibleConnections.map(({ id, index, p, native, name, subtitle, isDefault }) => {
          const dropping = dropTarget?.id === id ? `drop-${dropTarget.edge}` : '';
          return <div className={`provider-sort-row ${dragId === id ? 'is-dragging' : ''} ${dropping}`} key={`${agentTab}:${id}`} data-connection-id={id}>
            <button type="button" className="provider-drag-handle" aria-label={`调整 ${name} 的顺序`} title="拖动排序，或按 Alt + ↑ / ↓" disabled={sortingDisabled}
              onPointerDown={e => { if (sortingDisabled || e.button !== 0) return; e.currentTarget.setPointerCapture(e.pointerId); pointerDrag.current = { id, y: e.clientY, pointerId: e.pointerId, active: false }; }}
              onPointerMove={dragMove} onPointerUp={dragEnd} onPointerCancel={clearDrag} onLostPointerCapture={clearDrag}
              onKeyDown={e => { if (e.altKey && ['ArrowUp', 'ArrowDown'].includes(e.key)) { e.preventDefault(); move(id, e.key === 'ArrowUp' ? -1 : 1); } }}><GripVertical size={16}/></button>
            <button type="button" className="saved-provider" disabled={working || busy || !desktop} onClick={() => void run(async () => { if (p) setConfig(await call<Config>('get_provider_profile', { id })); else setNativeId(id); setMessage(''); })}>
              <span className={`provider-mark mark-${p?.providerId ?? native.providerId}`}>{id === '@local' ? <AgentIcon agent={agentTab}/> : <ProviderIcon provider={p?.officialAccount ? (agentTab === 'claude' ? 'anthropic' : 'openai') : p?.modelSource?.providerId ?? p?.providerId ?? native.providerId} name={name} mark={p?.modelSource?.mark ?? p?.name.slice(0, 1).toUpperCase() ?? native.mark}/>}</span>
              <span><strong>{name}</strong><small>{subtitle}</small></span><span className={`connection-state ${isDefault ? 'enabled' : ''}`}>{isDefault ? '新聊天默认' : p ? p.officialAccount ? '本机登录' : p.hasCredential ? '已配置' : '未保存密钥' : '本机登录'}</span><ChevronRight size={16}/>
            </button>
            <div className="provider-order-actions"><button type="button" className="icon-button" aria-label={`上移 ${name}`} title="上移" disabled={sortingDisabled || index === 0} onClick={() => move(id, -1)}><ArrowUp size={14}/></button><button type="button" className="icon-button" aria-label={`下移 ${name}`} title="下移" disabled={sortingDisabled || index === ids.length - 1} onClick={() => move(id, 1)}><ArrowDown size={14}/></button></div>
          </div>;
        })}</div>
        {!visibleConnections.length ? <p className="empty-filter">{connectionSearch.trim() ? '没有匹配的连接' : '暂无已配置连接'}</p> : null}
      </div>
    </div>}
    {login ? <div className="login-progress"><span>等待浏览器登录</span><button className="quiet-button" disabled={working} onClick={() => void run(async () => { const status = await call<{ loggedIn: boolean }>('official_account_status', { agent: login.agent }); setAccount(a => ({ ...a, [login.agent]: status })); if (status.loggedIn) { setLogin(null); setAdding(false); await updated(); setMessage('已登录'); } else setMessage('未完成登录'); })}>检查登录</button><button className="quiet-button" disabled={working} onClick={() => void run(async () => { await call('cancel_official_login', { agent: login.agent, loginId: login.id ?? null }); setLogin(null); setMessage('已取消'); })}>取消</button></div> : null}
    </div>
  </div>;
}
