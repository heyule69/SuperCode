import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, RefreshCw, Search } from 'lucide-react';
import { call, desktop } from './api';
import type { AgentProfile } from './types';
import { providerAgents } from './providerOrder';

interface ScanResult { path: string; profiles: AgentProfile[]; warnings: string[] }

export default function CCSwitchSettings({ agent, busy = false, runningConnections = {}, updated, back }: { agent: string; busy?: boolean; runningConnections?: Record<string, string[]>; updated: () => Promise<void>; back: () => void }) {
  const [path, setPath] = useState('');
  const [scanData, setScanData] = useState<ScanResult & { agent: string }>();
  const [selected, setSelected] = useState<string[]>([]);
  const [search, setSearch] = useState('');
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const request = useRef(0), alive = useRef(true), inFlight = useRef(false), databasePath = useRef('');
  const running = useRef(runningConnections);
  running.current = runningConnections;
  const inUse = (profile: AgentProfile) => running.current[profile.agent]?.includes(profile.id) ?? false;
  const current = (revision: number) => alive.current && revision === request.current;
  const agentName = providerAgents.find(item => item.id === agent)?.name ?? agent;
  const source = scanData?.agent === agent ? scanData.profiles : [];
  const warnings = scanData?.agent === agent ? scanData.warnings : [];
  const available = source.filter(p => !inUse(p));
  const selectedIds = available.filter(p => selected.includes(p.id)).map(p => p.id);
  async function run(action: (revision: number) => Promise<void>) {
    if (inFlight.current) return;
    inFlight.current = true; const revision = ++request.current;
    setWorking(true); setError(''); setMessage('');
    try { await action(revision); } catch (e) { if (current(revision)) setError(String(e)); } finally { if (current(revision)) { inFlight.current = false; setWorking(false); } }
  }
  async function scan(database: string | null, revision: number) {
    setScanData(undefined); setSelected([]);
    const result = await call<ScanResult>('scan_ccswitch', { path: database, agent });
    if (!current(revision)) return;
    const rows = result.profiles.filter(p => p.agent === agent);
    databasePath.current = result.path; setPath(result.path);
    setScanData({ ...result, agent, profiles: rows }); setSelected(rows.filter(p => p.current && !inUse(p)).map(p => p.id));
    setMessage(`${database ? '已读取' : '已自动识别 CC Switch'} · ${rows.length} 个 ${agentName} 可导入连接`);
  }
  useEffect(() => {
    alive.current = true; setSearch(''); setSelected([]); setScanData(undefined); setError(''); setMessage('');
    if (desktop) void run(revision => scan(databasePath.current.trim() || null, revision));
    return () => { alive.current = false; request.current++; inFlight.current = false; };
  }, [agent]);
  const visible = source.filter(p => `${p.name} ${p.agent} ${p.model ?? ''}`.toLowerCase().includes(search.toLowerCase()));
  const availableVisible = visible.filter(p => !inUse(p));
  async function importRows(ids: string[], revision: number) {
    const unlockedIds = source.filter(p => ids.includes(p.id) && !inUse(p)).map(p => p.id);
    if (!unlockedIds.length) return;
    const count = await call<number>('import_ccswitch', { path: path.trim() || null, agent, ids: unlockedIds });
    await updated(); if (current(revision)) setMessage(`已导入 ${count} 个 ${agentName} 连接`);
  }
  return <div className="cc-settings">
    <div className="provider-picker-heading"><h3>CC Switch 导入 · {agentName}</h3><button className="quiet-button" disabled={working} onClick={back}><ArrowLeft size={15} />返回</button></div>
    <div className="cc-path-row"><label htmlFor="cc-path">配置路径</label><input id="cc-path" value={path} disabled={working || busy} onChange={e => { databasePath.current = e.target.value; setPath(e.target.value); setScanData(undefined); setSelected([]); setMessage(''); setError(''); }} placeholder="自动识别 CC Switch 数据库或旧版配置" /><button className="quiet-button" disabled={!desktop || working || busy} onClick={() => void run(revision => scan(null, revision))}>自动识别</button><button className="quiet-button" disabled={!desktop || working || busy} onClick={() => void run(revision => scan(path.trim() || null, revision))}><RefreshCw size={14} className={working ? 'spin' : ''} />读取</button></div>
    <div className="cc-list-toolbar"><div className="search-field"><Search size={15} /><input aria-label="搜索导入配置" placeholder={`搜索 ${agentName} 连接`} value={search} onChange={e => setSearch(e.target.value)} /></div><span>已选 {selectedIds.length} / {source.length}</span><button className="quiet-button" disabled={working || busy || !availableVisible.length} onClick={() => setSelected(old => [...new Set([...old, ...availableVisible.map(p => p.id)])])}>{search ? '全选当前' : '全选'}</button><button className="quiet-button" disabled={working || busy || !selectedIds.length} onClick={() => setSelected([])}>清空</button></div>
    <div className="cc-source-list">{visible.map(p => <label className="cc-source-row" key={p.id}>
      <input aria-label={`导入 ${p.name} · ${p.agent}`} type="checkbox" checked={selectedIds.includes(p.id)} disabled={working || busy || inUse(p)} onChange={e => setSelected(old => e.target.checked ? [...old, p.id] : old.filter(id => id !== p.id))} />
      <span><strong>{p.name}</strong><small>{agentName}{p.model ? ` · ${p.model}` : ''}</small></span>
      {p.current ? <span className="connection-state">当前配置</span> : null}
      {inUse(p) ? <span className="connection-state">任务使用中</span> : null}
    </label>)}{!visible.length ? <p className="muted">{working ? '读取中…' : source.length ? '无匹配连接' : error ? '读取失败，请检查下方提示' : desktop ? `暂无 ${agentName} 可导入连接` : '请在桌面版读取配置'}</p> : null}</div>
    {warnings.length ? <details className="cc-scan-warnings"><summary>已跳过 {warnings.length} 个异常连接，其他连接可正常导入</summary><ul>{warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></details> : null}
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    <div className="provider-editor-footer"><span className="field-hint" role="status">{message || `仅导入 ${agentName} 连接；再次导入会更新对应连接`}{available.length < source.length ? '；任务使用中的连接暂不可更新' : ''}</span><div><button className="quiet-button" disabled={!desktop || working || busy || !available.length} onClick={() => void run(revision => importRows(available.map(p => p.id), revision))}>导入全部</button><button className="primary-button" disabled={!desktop || working || busy || !selectedIds.length} onClick={() => void run(revision => importRows(selectedIds, revision))}>{working ? '处理中…' : `导入所选 (${selectedIds.length})`}</button></div></div>
  </div>;
}
