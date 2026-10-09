import { useEffect, useMemo, useState } from 'react';
import { ChevronDown, RefreshCw } from 'lucide-react';
import { call, desktop } from './api';
import { compactNumber, number, usageDay, usageGroups, usageTotals, type UsageRecord } from './usage';
import { AgentIcon } from './AgentIcon';

export default function UsagePanel() {
  const [records, setRecords] = useState<UsageRecord[]>([]);
  const [range, setRange] = useState('7'); const [agent, setAgent] = useState('all');
  const [metric, setMetric] = useState<'tokens' | 'cost'>('tokens');
  const [group, setGroup] = useState<'model' | 'chat'>('model');
  const [day, setDay] = useState(''); const [expanded, setExpanded] = useState('');
  const [revision, setRevision] = useState(0); const [now, setNow] = useState(Date.now());
  const [loading, setLoading] = useState(false); const [error, setError] = useState('');
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; setLoading(true); setError('');
    call<{ records: UsageRecord[] }>('get_usage', { sessionId: null }).then(value => {
      if (!disposed) { setRecords(value.records); setNow(Date.now()); }
    }).catch(e => { if (!disposed) setError(String(e)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [revision]);
  const visible = useMemo(() => records.filter(r => (agent === 'all' || r.agent === agent) && (range === 'all' || r.at >= now / 1000 - Number(range) * 86400)), [records, agent, range, now]);
  const totals = useMemo(() => usageTotals(visible), [visible]);
  const activeMetric = totals.priced ? metric : 'tokens';
  const daily = useMemo(() => {
    const days = new Map<string, UsageRecord[]>();
    for (const record of visible) { const key = usageDay(record.at); days.set(key, [...(days.get(key) ?? []), record]); }
    if (range !== 'all') {
      const date = new Date(now - Number(range) * 86400000); date.setHours(0, 0, 0, 0);
      while (date.getTime() <= now) { const key = usageDay(date.getTime() / 1000); if (!days.has(key)) days.set(key, []); date.setDate(date.getDate() + 1); }
    }
    return [...days].sort(([a], [b]) => a.localeCompare(b)).slice(-31).map(([date, items]) => ({ date, ...usageTotals(items) }));
  }, [visible, range, now]);
  const peak = Math.max(activeMetric === 'cost' ? 0.01 : 1, ...daily.map(d => d[activeMetric]));
  const groups = useMemo(() => usageGroups(day ? visible.filter(r => usageDay(r.at) === day) : visible, group), [visible, group, day]);
  const metricValue = (value: number) => activeMetric === 'cost' ? `$${value.toFixed(2)}` : compactNumber(value);
  const changeRange = (value: string) => { setRange(value); setDay(''); setExpanded(''); };
  return <div className="client-settings-page meter-page">
    <div className="settings-page-heading meter-heading"><h2>用量</h2><div className="meter-actions">
      <label className="meter-select"><select aria-label="Agent 筛选" value={agent} onChange={e => { setAgent(e.target.value); setDay(''); }}><option value="all">所有 Agent</option><option value="codex">Codex</option><option value="claude">Claude Code</option><option value="opencode">OpenCode</option><option value="pi">Pi</option></select><ChevronDown size={13}/></label>
      <label className="meter-select"><select aria-label="用量时间范围" value={range} onChange={e => changeRange(e.target.value)}><option value="1">近 24 小时</option><option value="7">近 7 天</option><option value="30">近 30 天</option><option value="all">全部记录</option></select><ChevronDown size={13}/></label>
      <button className="icon-button" aria-label="刷新用量" disabled={loading || !desktop} onClick={() => setRevision(v => v + 1)}><RefreshCw size={15}/></button>
    </div></div>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    <div className="meter-stats">{[['总 Token', totals.tokens], ['输入', totals.input], ['输出', totals.output], ['缓存读取', totals.cached]].map(([label, value]) => <div key={label}><span>{label}</span><strong title={number(Number(value))}>{compactNumber(Number(value))}</strong></div>)}</div>
    <section className="meter-section"><div className="meter-section-heading"><h3>使用趋势</h3><div className="meter-segmented" aria-label="趋势指标"><button aria-pressed={activeMetric === 'tokens'} onClick={() => setMetric('tokens')}>Token</button>{totals.priced ? <button aria-pressed={activeMetric === 'cost'} onClick={() => setMetric('cost')}>费用</button> : null}</div></div>
      <div className="meter-chart"><div className="meter-chart-caption"><span>{activeMetric === 'cost' ? 'Agent 返回的费用 · USD' : '每日 Token'}</span>{day ? <button onClick={() => setDay('')}>{day.slice(5).replace('-', '/')} · 清除筛选</button> : <span>点击日期查看明细</span>}</div>
        {visible.length ? <div className="meter-chart-scroll"><div className="meter-chart-plot" style={{ minWidth: daily.length > 10 ? daily.length * 23 + 44 : undefined }}><div className="meter-axis"><span>{metricValue(peak)}</span><span>{metricValue(peak / 2)}</span><span>0</span></div><div className="meter-grid"><i/><i/><i/></div><div className="meter-bars">{daily.map(d => <button key={d.date} aria-label={`${d.date}：${metricValue(d[activeMetric])}`} aria-pressed={day === d.date} title={`${d.date} · ${activeMetric === 'cost' && !d.priced ? '未返回费用' : metricValue(d[activeMetric])}`} onClick={() => { setDay(day === d.date ? '' : d.date); setExpanded(''); }}><span className="meter-bar-space"><i style={{ height: d[activeMetric] ? `${Math.max(2, d[activeMetric] / peak * 100)}%` : '0' }}/></span><small>{d.date.slice(5).replace('-', '/')}</small></button>)}</div></div></div> : <div className="meter-empty">{loading ? '正在读取用量…' : '暂无用量记录'}</div>}
      </div>
    </section>
    <section className="meter-section"><div className="meter-section-heading"><h3>{day ? `${day.slice(5).replace('-', '/')} 明细` : '用量明细'}</h3><div className="meter-segmented" aria-label="明细分组"><button aria-pressed={group === 'model'} onClick={() => { setGroup('model'); setExpanded(''); }}>按模型</button><button aria-pressed={group === 'chat'} onClick={() => { setGroup('chat'); setExpanded(''); }}>按对话</button></div></div>
      <div className="meter-details"><div className="meter-detail-head"><span>{group === 'model' ? '模型' : '对话'}</span><span>{activeMetric === 'cost' ? '费用 · USD' : 'Token'}</span><span>最近使用</span></div>{groups.map(item => <div className="meter-detail-group" key={item.id}><button className="meter-detail-row" aria-expanded={expanded === item.id} onClick={() => setExpanded(expanded === item.id ? '' : item.id)}><span className="meter-detail-name"><AgentIcon agent={item.agent}/><span><strong>{item.title}</strong><small>{group === 'model' ? `${({ claude: 'Claude Code', codex: 'Codex', opencode: 'OpenCode', pi: 'Pi' } as Record<string,string>)[item.agent]} · ${item.records.length} 条记录` : [...item.models].join(' · ')}</small></span></span><span>{activeMetric === 'cost' ? item.totals.priced ? `$${item.totals.cost.toFixed(4)}` : '—' : compactNumber(item.totals.tokens)}</span><span>{new Date(item.at * 1000).toLocaleString(undefined, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span></button>{expanded === item.id ? <div className="meter-detail-breakdown"><span>输入 <b>{number(item.totals.input)}</b></span><span>输出 <b>{number(item.totals.output)}</b></span><span>缓存读取 <b>{number(item.totals.cached)}</b></span>{item.totals.priced ? <span>费用 <b>${item.totals.cost.toFixed(4)} USD</b></span> : null}</div> : null}</div>)}{!groups.length ? <p className="meter-empty">{loading ? '正在读取用量…' : '这个范围暂无记录'}</p> : null}</div>
    </section><p className="meter-note">显示最近 1000 条记录。缓存单独列出，不重复计入总量；费用仅采用 Agent 返回的数据。</p>
  </div>;
}
