import { memo, useDeferredValue, useEffect, useMemo, useState } from 'react';
import { RefreshCw, Search, X } from 'lucide-react';
import { call, desktop } from './api';
import { AgentIcon, ProviderIcon } from './AgentIcon';
import { resetLabel, usePlatformUsage, type QuotaConnection } from './platformUsage';
import { agentNames, QuotaConnections, quotaConnectionKey } from './QuotaConnections';
import type { AgentProfile } from './types';

const QuotaDetail = memo(function QuotaDetail({ connection, revision }: { connection: QuotaConnection; revision: string }) {
  const { value, loading, refresh } = usePlatformUsage(connection.agent, connection.connectionId, revision);
  const source = connection.source;
  return <section className="quota-detail" aria-label={`${connection.connectionName} 额度`}>
    <div className="quota-detail-heading"><span className="quota-brand"><ProviderIcon provider={source.providerId} name={connection.connectionName} mark={source.mark}/></span><div><h3>{connection.connectionName}</h3><p>{value?.planName || source.planName || source.providerName}</p></div><button className="icon-button" aria-label="刷新所选平台额度" disabled={loading || !desktop} onClick={() => void refresh(true)}><RefreshCw size={15} className={loading ? 'spin' : undefined}/></button></div>
    <div className="quota-meta"><span><AgentIcon agent={connection.agent}/>{agentNames[connection.agent]}</span><span role="status">{loading && value ? '更新中…' : value ? `${new Date(value.queriedAt * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })} 更新` : null}</span></div>
    <div className="quota-detail-scroll" aria-busy={loading}>
    {value ? <>
      {value.windows.length ? <div className="quota-allowances">{value.windows.map((window, index) => <div className="quota-window" key={`${window.label}:${index}`}><div className="quota-window-heading"><h4>{window.label}</h4>{resetLabel(window.resetAt) ? <span>{resetLabel(window.resetAt)}</span> : null}</div><div className="quota-remaining"><strong>{Math.floor(window.remainingPercent)}<small>%</small></strong><span>剩余</span></div><progress className={window.remainingPercent <= 20 ? 'quota-low' : ''} max={100} value={window.remainingPercent} aria-label={`${window.label}剩余额度`}/></div>)}</div> : null}
      {value.balances.length ? <div className="quota-balances">{value.balances.map((balance, index) => <div className="quota-balance" key={`${balance.label}:${index}`}><span>{balance.label}</span><strong>{balance.currency === 'CNY' ? '¥' : balance.currency === 'USD' ? '$' : ''}{balance.amount.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}<small>{balance.currency}</small></strong></div>)}</div> : null}
      <p className="meter-note">数据由当前连接的平台提供。</p>
    </> : loading ? <div className="quota-loading" role="status"><span className="quota-loading-label">正在读取额度…</span><div className="quota-skeleton-cards" aria-hidden="true">{[0, 1].map(index => <div className="quota-skeleton-card" key={index}><i/><b/><i/></div>)}</div></div> : <div className="quota-unavailable"><strong>暂无可读取的额度数据</strong><p>该平台未提供可读取的数据，或当前连接未能获取数据。</p><button className="quiet-button" disabled={!desktop} onClick={() => void refresh(true)}><RefreshCw size={13}/>重新读取</button></div>}
    </div>
  </section>;
});

export default function QuotaPanel({ profiles, officialAgents }: { profiles: AgentProfile[]; officialAgents: string[] }) {
  const [connections, setConnections] = useState<QuotaConnection[]>([]);
  const [selected, setSelected] = useState(''); const [search, setSearch] = useState('');
  const query = useDeferredValue(search.trim().toLocaleLowerCase());
  const [reload, setReload] = useState(0); const [loading, setLoading] = useState(false); const [error, setError] = useState('');
  const revision = useMemo(() => JSON.stringify([profiles, officialAgents]), [profiles, officialAgents]);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; setLoading(true); setError('');
    call<QuotaConnection[]>('get_quota_connections').then(value => { if (!disposed) setConnections(value); }).catch(e => { if (!disposed) setError(String(e)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [revision, reload]);
  const indexed = useMemo(() => connections.map(connection => ({ connection, key: quotaConnectionKey(connection), text: `${connection.connectionName} ${connection.source.providerName} ${connection.source.planName ?? ''} ${agentNames[connection.agent] ?? connection.agent}`.toLocaleLowerCase() })), [connections]);
  const byKey = useMemo(() => new Map(indexed.map(item => [item.key, item.connection])), [indexed]);
  const active = byKey.get(selected) ?? connections[0];
  const matching = useMemo(() => indexed.filter(item => item.text.includes(query)).map(item => item.connection), [indexed, query]);
  return <div className="client-settings-page meter-page quota-page"><div className="settings-page-heading meter-heading"><div><h2>额度</h2><p>选择连接，查看平台提供的套餐额度与余额。</p></div><button className="quiet-button" aria-label="刷新连接列表" disabled={loading || !desktop} onClick={() => setReload(v => v + 1)}><RefreshCw size={14} className={loading ? 'spin' : undefined}/>刷新连接</button></div>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    {connections.length ? <div className="quota-layout"><aside className="quota-connections" aria-label="连接的平台"><div className="quota-list-heading"><span>连接的平台</span><small>{query ? `${matching.length} / ${connections.length}` : connections.length}</small></div><label className="quota-search"><Search size={14}/><input aria-label="搜索额度连接" placeholder="搜索连接或平台" value={search} onChange={e => setSearch(e.target.value)}/>{search ? <button type="button" className="icon-button" aria-label="清除连接搜索" onClick={() => setSearch('')}><X size={13}/></button> : null}</label><QuotaConnections connections={matching} activeKey={active ? quotaConnectionKey(active) : ''} choose={setSelected}/></aside>{active ? <QuotaDetail key={quotaConnectionKey(active)} connection={active} revision={revision}/> : null}</div> : <div className="quota-unavailable"><strong>{loading ? '正在读取连接…' : '暂无平台连接'}</strong>{!loading ? <p>在模型供应商页面配置连接后，会显示在这里。</p> : null}</div>}
  </div>;
}
