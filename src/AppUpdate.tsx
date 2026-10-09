import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Download, RefreshCw, X } from 'lucide-react';
import { call, desktop } from './api';
import './app-update.css';

export interface AppUpdateStatus {
  currentVersion: string; latestVersion: string | null; notes: string;
  phase: 'idle' | 'checking' | 'available' | 'current' | 'unpublished' | 'unsupported' | 'error' | 'downloading' | 'ready' | 'installing';
  progress: number; automatic: boolean; checkedAt: number | null; error: string | null; releaseUrl: string;
}
function useUpdateStatus() {
  const [status, setStatus] = useState<AppUpdateStatus | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!desktop) return;
    let disposed = false, unlisten: (() => void) | undefined, receivedEvent = false;
    void listen<AppUpdateStatus>('app-update', event => {
      receivedEvent = true; if (!disposed) setStatus(event.payload);
    }).then(async off => {
      if (disposed) { off(); return; } unlisten = off;
      const value = await call<AppUpdateStatus>('app_update_status');
      if (!disposed && !receivedEvent) setStatus(value);
    }).catch(e => { if (!disposed) setError(String(e)); });
    return () => { disposed = true; unlisten?.(); };
  }, []);
  async function action(command: string, args: Record<string, unknown> = {}) {
    setError('');
    try { const value = await call<AppUpdateStatus | undefined>(command, args); if (value) setStatus(value); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  }
  return { status, error, action };
}
export function AppUpdateNotice({ open }: { open: () => void }) {
  const { status } = useUpdateStatus();
  const [dismissed, setDismissed] = useState('');
  if (!status?.latestVersion || !['available', 'ready'].includes(status.phase) || dismissed === status.latestVersion) return null;
  return <aside className="app-update-notice" aria-label="软件更新"><span>SuperCode {status.latestVersion} 可更新</span><button onClick={open}>查看更新</button><button className="icon-button" aria-label="暂不更新" onClick={() => setDismissed(status.latestVersion!)}><X size={13}/></button></aside>;
}
export default function AppUpdateSettings({ busy }: { busy: boolean }) {
  const { status, error, action } = useUpdateStatus();
  const [pending, setPending] = useState(false);
  async function run(command: string, args?: Record<string, unknown>) { if (pending) return; setPending(true); try { await action(command, args); } finally { setPending(false); } }
  const working = pending || !!status && ['checking', 'downloading', 'installing'].includes(status.phase);
  const text = !desktop ? '请在桌面版中检查更新。' : !status ? '正在读取更新状态…'
    : status.phase === 'checking' ? '正在检查更新…' : status.phase === 'available' ? `发现新版本 ${status.latestVersion}`
    : status.phase === 'ready' ? `版本 ${status.latestVersion} 已准备好` : status.phase === 'downloading' ? `正在下载 ${status.progress}%`
    : status.phase === 'installing' ? '正在重启并更新…' : status.phase === 'current' ? '已是最新版本'
    : status.phase === 'unpublished' ? '暂无已发布的新版本' : status.phase === 'unsupported' ? '请从 GitHub 下载适用版本'
    : status.phase === 'error' ? '暂时无法检查更新' : '从 GitHub Releases 获取更新';
  return <section className="app-update-settings" aria-label="软件更新">
    <div className="app-update-heading"><div><strong>软件更新</strong><p role="status">{text}</p></div><button className="quiet-button" disabled={!desktop || working} onClick={() => void run('check_app_update', { force: true })}><RefreshCw size={14}/>检查更新</button></div>
    <label className="app-update-automatic"><input type="checkbox" checked={status?.automatic ?? true} disabled={!desktop || !status || working} onChange={e => void run('set_automatic_app_updates', { enabled: e.target.checked })}/>自动检查更新</label>
    {status?.notes && ['available', 'ready', 'downloading'].includes(status.phase) ? <p className="app-update-notes">{status.notes}</p> : null}
    {status?.phase === 'downloading' ? <progress aria-label="更新下载进度" value={status.progress} max={100}/> : null}
    {status?.phase === 'available' ? <button className="primary-button" disabled={working} onClick={() => void run('download_app_update')}><Download size={14}/>下载更新</button> : null}
    {status?.phase === 'ready' ? <div><button className="primary-button" disabled={working || busy} onClick={() => void run('install_app_update')}>重启并更新</button><p className="muted">{busy ? '请先完成正在运行的任务。' : '聊天和配置会保留。'}</p></div> : null}
    {error || status?.error ? <p className="app-update-error" role="alert">{error || status?.error}</p> : null}
    <button className="app-update-link" disabled={!desktop} onClick={() => void action('open_external_link', { url: status?.releaseUrl ?? 'https://github.com/heyule69/SuperCode/releases' })}>查看 GitHub 版本记录</button>
  </section>;
}
