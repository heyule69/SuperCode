import { useEffect, useState } from 'react';
import { ArrowLeft, RefreshCw, Search } from 'lucide-react';
import { call, desktop } from './api';
import type { AgentProfile } from './types';

export default function CCSwitchSettings({ busy, updated, back }: { busy: boolean; updated: () => Promise<void>; back: () => void }) {
  const [path, setPath] = useState('');
  const [source, setSource] = useState<AgentProfile[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [search, setSearch] = useState('');
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  async function run(action: () => Promise<void>) {
    setWorking(true); setError(''); setMessage('');
    try { await action(); } catch (e) { setError(String(e)); } finally { setWorking(false); }
  }
  async function scan(database: string | null = path.trim() || null) {
    const rows = await call<AgentProfile[]>('scan_ccswitch', { path: database });
    setSource(rows); setSelected(rows.filter(p => p.current).map(p => p.id));
    if (!rows.length) setMessage('未找到配置');
  }
  useEffect(() => { if (desktop) void run(() => scan(null)); }, []);
  const visible = source.filter(p => `${p.name} ${p.agent} ${p.model ?? ''}`.toLowerCase().includes(search.toLowerCase()));
  async function importRows(ids: string[]) {
    const count = await call<number>('import_ccswitch', { path: path.trim() || null, ids });
    await updated(); setMessage(`已导入 ${count} 个连接`);
  }
  return <div className="cc-settings">
    <div className="provider-picker-heading"><h3>CC Switch 导入</h3><button className="quiet-button" disabled={working} onClick={back}><ArrowLeft size={15} />返回</button></div>
    <div className="cc-path-row"><label htmlFor="cc-path">数据库</label><input id="cc-path" value={path} disabled={working || busy} onChange={e => { setPath(e.target.value); setSource([]); setSelected([]); setMessage(''); }} placeholder="自动查找 ~/.cc-switch/cc-switch.db" /><button className="quiet-button" disabled={!desktop || working || busy} onClick={() => void run(() => scan())}><RefreshCw size={14} className={working ? 'spin' : ''} />读取</button></div>
    <div className="cc-list-toolbar"><div className="search-field"><Search size={15} /><input aria-label="搜索导入配置" placeholder="搜索连接" value={search} onChange={e => setSearch(e.target.value)} /></div><span>已选 {selected.length} / {source.length}</span><button className="quiet-button" disabled={working || busy || !visible.length} onClick={() => setSelected(old => [...new Set([...old, ...visible.map(p => p.id)])])}>{search ? '全选当前' : '全选'}</button><button className="quiet-button" disabled={working || busy || !selected.length} onClick={() => setSelected([])}>清空</button></div>
    <div className="cc-source-list">{visible.map(p => <label className="cc-source-row" key={p.id}>
      <input aria-label={`导入 ${p.name} · ${p.agent}`} type="checkbox" checked={selected.includes(p.id)} disabled={working || busy} onChange={e => setSelected(old => e.target.checked ? [...old, p.id] : old.filter(id => id !== p.id))} />
      <span><strong>{p.name}</strong><small>{p.agent === 'claude' ? 'Claude Code' : 'Codex'}{p.model ? ` · ${p.model}` : ''}</small></span>
      {p.current ? <span className="connection-state">当前配置</span> : null}
    </label>)}{!visible.length ? <p className="muted">{working ? '读取中…' : source.length ? '无匹配连接' : desktop ? '暂无配置' : '请在桌面版读取配置'}</p> : null}</div>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    <div className="provider-editor-footer"><span className="field-hint" role="status">{message || '仅导入连接；再次导入会更新源配置对应的连接'}</span><div><button className="quiet-button" disabled={!desktop || working || busy || !source.length} onClick={() => void run(() => importRows(source.map(p => p.id)))}>导入全部</button><button className="primary-button" disabled={!desktop || working || busy || !selected.length} onClick={() => void run(() => importRows(selected))}>{working ? '处理中…' : `导入所选 (${selected.length})`}</button></div></div>
  </div>;
}
