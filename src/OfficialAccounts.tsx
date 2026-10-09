import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, Check, ExternalLink, LoaderCircle, Pencil, Plus, RefreshCw, Trash2 } from 'lucide-react';
import { ProviderIcon } from './AgentIcon';
import { call, desktop } from './api';
import { officialProvider } from './providerCapabilities';

export interface OfficialAccount { id: string; agent: string; accountId?: string | null; name: string; loggedIn: boolean; current: boolean; plan?: string | null; error?: string }
type Pending = { agent: string; accountId: string; name: string };
const storageKey = 'supercode.officialAccountLogin';
function pendingLogin(): Pending | null { try { return JSON.parse(sessionStorage.getItem(storageKey) ?? 'null'); } catch { return null; } }

export default function OfficialAccounts({ agent, busy, back, updated }: { agent: string; busy: boolean; back: () => void; updated: (resetAgent?: string) => Promise<void> }) {
  const [rows, setRows] = useState<OfficialAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState(false);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState('');
  const [pending, setPending] = useState<Pending | null>(pendingLogin);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [editing, setEditing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const revision = useRef(0);
  const provider = officialProvider(agent)!;
  useEffect(() => { if (pending) sessionStorage.setItem(storageKey, JSON.stringify(pending)); else sessionStorage.removeItem(storageKey); }, [pending]);
  useEffect(() => {
    const version = ++revision.current;
    setRows([]); setLoading(true); setAdding(false); setEditing(null); setDeleting(null); setError(''); setMessage('');
    if (!desktop) { setLoading(false); return; }
    void call<OfficialAccount[]>('list_official_accounts', { agent, force: false }).then(result => { if (version === revision.current) setRows(result); }).catch(e => { if (version === revision.current) setError(String(e)); }).finally(() => { if (version === revision.current) setLoading(false); });
    return () => { ++revision.current; };
  }, [agent]);
  async function refresh() { setRows(await call<OfficialAccount[]>('list_official_accounts', { agent, force: true })); }
  async function run(action: () => Promise<void>) {
    setWorking(true); setError(''); setMessage('');
    try { await action(); } catch (e) { setError(String(e)); } finally { setWorking(false); }
  }
  const disabled = !desktop || busy || working;
  const ownPending = pending?.agent === agent ? pending : null;
  return <div className="official-accounts">
    <div className="provider-editor-heading"><button className="quiet-button" disabled={working} onClick={back}><ArrowLeft size={15}/>所有连接</button><button className="quiet-button" disabled={disabled || loading} onClick={() => void run(async () => { await refresh(); await updated(); })}><RefreshCw size={15}/>刷新账号</button></div>
    <div className="official-account-heading"><span className="provider-mark"><ProviderIcon provider={provider.id}/></span><div><h3>{provider.name}</h3><p>添加多个账号，分别用于不同聊天。</p></div><button className="primary-button" disabled={disabled || !!pending} onClick={() => { setAdding(true); setName(''); setError(''); }}><Plus size={15}/>添加账号</button></div>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    {message ? <p className="provider-feedback" role="status"><Check size={15}/>{message}</p> : null}
    {adding ? <form className="official-account-add" onSubmit={e => { e.preventDefault(); void run(async () => { const result = await call<{ accountId: string; message: string }>('start_official_account_login', { agent, name: name.trim() }); setPending({ agent, name: name.trim(), accountId: result.accountId }); setAdding(false); setMessage(result.message); }); }}>
      <label htmlFor="official-account-name">账号名称</label><input id="official-account-name" autoFocus placeholder="例如：个人账号、工作账号" value={name} disabled={disabled} onChange={e => setName(e.target.value)} maxLength={100}/><div><button type="button" className="quiet-button" disabled={working} onClick={() => setAdding(false)}>取消</button><button className="primary-button" disabled={disabled || !name.trim()}><ExternalLink size={15}/>前往官方登录</button></div>
    </form> : null}
    {ownPending ? <div className="official-account-pending" role="status"><LoaderCircle size={18} className="spin"/><div><strong>{ownPending.name}</strong><span>等待浏览器登录</span></div><button className="quiet-button" disabled={disabled} onClick={() => void run(async () => { const status = await call<{ loggedIn: boolean; isDefault?: boolean }>('finish_official_account_login', { accountId: ownPending.accountId }); if (!status.loggedIn) { setMessage('尚未完成登录，请在浏览器完成后再检查'); return; } setPending(null); await refresh(); await updated(status.isDefault ? agent : undefined); setMessage('账号已添加'); })}>检查登录</button><button className="quiet-button" disabled={disabled} onClick={() => void run(async () => { await call('cancel_official_account_login', { accountId: ownPending.accountId }); setPending(null); setMessage('已取消登录'); })}>取消</button></div> : null}
    {loading ? <p className="muted"><LoaderCircle className="spin" size={15}/>正在检查账号…</p> : <div className="official-account-list">{rows.map(row => <div className="official-account-row" key={row.id}>
      <span className={`account-status-dot ${row.loggedIn ? 'online' : ''}`}/><div className="official-account-details">{editing && editing === row.accountId ? <form onSubmit={e => { e.preventDefault(); void run(async () => { await call('rename_official_account', { agent, accountId: row.accountId, name: name.trim() }); setEditing(null); await refresh(); await updated(); }); }}><input aria-label="修改账号名称" autoFocus value={name} disabled={disabled} onChange={e => setName(e.target.value)}/><button className="quiet-button" disabled={disabled || !name.trim()}>保存</button><button type="button" className="quiet-button" disabled={working} onClick={() => setEditing(null)}>取消</button></form> : <><strong>{row.name}{row.current && row.loggedIn ? <small>新聊天默认</small> : null}</strong><span>{row.error ?? (row.loggedIn ? `已登录${row.plan ? ` · ${row.plan}` : ''}` : '未登录')}{!row.accountId ? ' · 跟随本机 CLI 账号' : ''}</span></>}</div>
      {deleting && deleting === row.accountId ? <div className="account-actions"><span>移除账号？</span><button className="quiet-button" disabled={working} onClick={() => setDeleting(null)}>取消</button><button className="danger-button" disabled={disabled} onClick={() => void run(async () => { await call('remove_official_account', { agent, accountId: row.accountId }); setDeleting(null); await refresh(); await updated(); })}>移除</button></div> : <div className="account-actions"><button className="quiet-button" disabled={disabled || !row.loggedIn || row.current} onClick={() => void run(async () => { await call('use_official_account', { agent, accountId: row.accountId ?? null }); await refresh(); await updated(agent); setMessage('已设为新聊天默认'); })}>{row.current && row.loggedIn ? <><Check size={14}/>默认账号</> : '设为默认'}</button>{row.accountId ? <><button className="icon-button" aria-label={`重命名 ${row.name}`} disabled={disabled} onClick={() => { setName(row.name); setEditing(row.accountId!); }}><Pencil size={15}/></button><button className="icon-button" aria-label={`移除 ${row.name}`} disabled={disabled} onClick={() => setDeleting(row.accountId!)}><Trash2 size={15}/></button></> : null}</div>}
    </div>)}</div>}
    <p className="official-account-note">新增账号独立保存登录。已有聊天继续使用原来的账号。</p>
  </div>;
}
