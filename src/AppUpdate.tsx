import { lazy, Suspense, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { AlertCircle, Check, Download, ExternalLink, RefreshCw, X } from 'lucide-react';
import { call, desktop } from './api';
import './app-update.css';

const UpdateReleaseNotes = lazy(() => import('./UpdateReleaseNotes'));

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
export function AppUpdateNotice({ open, busy = false, blocked = false, dialogChanged }: { open: () => void; busy?: boolean; blocked?: boolean; dialogChanged?: (open: boolean) => void }) {
  const { status, error, action } = useUpdateStatus();
  const [dismissed, setDismissed] = useState('');
  const [noticeDismissed, setNoticeDismissed] = useState('');
  const [dialogVersion, setDialogVersion] = useState('');
  const [pending, setPending] = useState(false);
  const panel = useRef<HTMLElement>(null);
  const operation = useRef(false);
  const titleId = useId(), descriptionId = useId();
  const available = !!status?.latestVersion && ['available', 'ready'].includes(status.phase);
  useEffect(() => {
    if (available && !blocked && dismissed !== status!.latestVersion) setDialogVersion(status!.latestVersion!);
  }, [available, blocked, dismissed, status?.latestVersion]);
  const visible = !!dialogVersion && !blocked && status?.latestVersion === dialogVersion && !['idle', 'current', 'unpublished', 'unsupported'].includes(status.phase);
  useEffect(() => {
    dialogChanged?.(visible);
    return () => dialogChanged?.(false);
  }, [visible, dialogChanged]);
  useEffect(() => {
    if (!visible) return;
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLButtonElement>('[data-update-dismiss]')?.focus();
    return () => { requestAnimationFrame(() => {
      if (previous?.isConnected && !panel.current && !document.querySelector('[aria-modal="true"]')) previous.focus({ preventScroll: true });
    }); };
  }, [visible]);
  function dismiss() { setDismissed(dialogVersion); setDialogVersion(''); }
  async function run(command: string) {
    if (operation.current) return;
    operation.current = true; setPending(true);
    try { await action(command); } finally { operation.current = false; setPending(false); }
  }
  function keys(event: React.KeyboardEvent<HTMLElement>) {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); dismiss(); }
    if (event.key !== 'Tab') return;
    const items = [...panel.current!.querySelectorAll<HTMLElement>('button:not(:disabled),a[href]')];
    const first = items[0], last = items[items.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }
  const working = pending || !!status && ['checking', 'downloading', 'installing'].includes(status.phase);
  return <>
    {available && noticeDismissed !== status!.latestVersion ? <aside className="app-update-notice" aria-label="软件更新"><span>SuperCode {status!.latestVersion} 可更新</span><button onClick={() => { setDismissed(status!.latestVersion!); setDialogVersion(''); open(); }}>查看更新</button><button className="icon-button" aria-label="暂不更新" onClick={() => { setDismissed(status!.latestVersion!); setNoticeDismissed(status!.latestVersion!); setDialogVersion(''); }}><X size={13}/></button></aside> : null}
    {visible ? createPortal(<div className="modal-backdrop app-update-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) dismiss(); }}>
      <section ref={panel} className="modal app-update-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId} onKeyDown={keys}>
        <div className="modal-header"><h2 id={titleId}>发现 SuperCode 新版本</h2><button type="button" className="icon-button" aria-label="关闭更新提示" data-update-dismiss onClick={dismiss}><X size={18}/></button></div>
        <p id={descriptionId} className="app-update-version">当前版本 {status!.currentVersion}<span aria-hidden="true"> → </span><strong>新版本 {status!.latestVersion}</strong></p>
        {status!.notes ? <div className="app-update-dialog-notes"><h3>更新内容</h3><div className="app-update-notes"><Suspense fallback={<p>正在读取更新内容…</p>}><UpdateReleaseNotes notes={status!.notes} openLink={url => void action('open_external_link', { url })}/></Suspense></div></div> : null}
        {status!.phase === 'downloading' ? <div className="app-update-dialog-progress" role="status"><p>正在下载更新 {status!.progress}%</p><progress aria-label="更新下载进度" value={status!.progress} max={100}/></div> : null}
        {status!.phase === 'installing' ? <p role="status">正在重启并更新…</p> : null}
        {error || status!.error ? <p className="app-update-dialog-error" role="alert">{error || status!.error}</p> : null}
        <p className="app-update-dialog-note">{status!.phase === 'ready' ? busy ? '更新已下载，请先完成正在运行的任务再重启。' : '更新已下载，重启后完成安装。聊天和配置会保留。' : '下载更新期间可以继续工作，安装前需要重启软件。'}</p>
        <div className="modal-actions"><button type="button" className="quiet-button" onClick={dismiss}>稍后提醒</button>
          {status!.phase === 'ready' ? <button type="button" className="primary-button" disabled={working || busy} onClick={() => void run('install_app_update')}>重启并更新</button>
            : status!.phase === 'error' ? <button type="button" className="primary-button" disabled={working} onClick={() => void run('download_app_update')}>重试下载</button>
            : <button type="button" className="primary-button" disabled={working || status!.phase !== 'available'} onClick={() => void run('download_app_update')}><Download size={15}/>{status!.phase === 'downloading' ? '正在下载…' : '下载更新'}</button>}
        </div>
      </section>
    </div>, document.body) : null}
  </>;
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
  const showNotes = !!status?.notes && ['available', 'ready', 'downloading'].includes(status.phase);
  const StateIcon = status?.phase === 'error' ? AlertCircle : status && ['current', 'ready'].includes(status.phase) ? Check
    : status && ['available', 'downloading'].includes(status.phase) ? Download : RefreshCw;
  return <section className="app-update-settings" aria-label="软件更新">
    <div className="app-update-heading">
      <div className="app-update-heading-copy">
        <span className="app-update-state-icon" aria-hidden="true"><StateIcon size={18} className={status?.phase === 'checking' ? 'spin' : undefined}/></span>
        <div><h3>软件更新</h3><p role="status">{text}</p></div>
      </div>
      <button className="quiet-button" disabled={!desktop || working} onClick={() => void run('check_app_update', { force: true })}><RefreshCw size={14}/>检查更新</button>
    </div>
    {showNotes ? <div className="app-update-release">
      <h4>更新内容</h4>
      <div className="app-update-notes"><Suspense fallback={<p>正在读取更新内容…</p>}><UpdateReleaseNotes notes={status!.notes} openLink={url => void action('open_external_link', { url })}/></Suspense></div>
    </div> : null}
    {status?.phase === 'downloading' ? <div className="app-update-progress"><progress aria-label="更新下载进度" value={status.progress} max={100}/></div> : null}
    {error || status?.error ? <p className="app-update-error" role="alert">{error || status?.error}</p> : null}
    <div className="app-update-actions">
      {status?.phase === 'available' ? <button className="primary-button" disabled={working} onClick={() => void run('download_app_update')}><Download size={15}/>下载更新</button> : null}
      {status?.phase === 'ready' ? <button className="primary-button" disabled={working || busy} onClick={() => void run('install_app_update')}>重启并更新</button> : null}
      <button className="app-update-link" disabled={!desktop} onClick={() => void action('open_external_link', { url: status?.releaseUrl ?? 'https://github.com/heyule69/SuperCode/releases' })}>查看 GitHub 版本记录<ExternalLink size={13}/></button>
    </div>
    {status?.phase === 'ready' ? <p className="app-update-restart-note">{busy ? '请先完成正在运行的任务。' : '聊天和配置会保留。'}</p> : null}
    <label className="app-update-automatic"><span>自动检查更新<small>启动约 10 秒后检查，之后每 6 小时检查一次；发现新版本时弹窗提醒。</small></span><input type="checkbox" role="switch" aria-label="自动检查更新" checked={status?.automatic ?? true} disabled={!desktop || !status || working} onChange={e => void run('set_automatic_app_updates', { enabled: e.target.checked })}/></label>
  </section>;
}
