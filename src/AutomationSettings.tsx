import { useEffect, useRef, useState } from 'react';
import { Check, Download, Globe, Monitor, RefreshCw, ShieldCheck, X } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { call, desktop } from './api';
import { automationDownload, automationKinds, automationWorking, mergeAutomationProgress, type AutomationKind, type AutomationStatus } from './automation';
import './automation.css';

const initial: AutomationStatus[] = automationKinds.map(kind => ({ kind, phase: 'notInstalled', message: '尚未安装', installation: null, received: null, total: null, enabled: false }));
export default function AutomationSettings({ busy }: { busy: boolean }) {
  const [items, setItems] = useState(initial);
  const [loading, setLoading] = useState(desktop);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState('');
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    if (!desktop) return () => { mounted.current = false; };
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await listen<AutomationStatus>('automation-progress', e => {
          if (!disposed) setItems(items => mergeAutomationProgress(items, e.payload));
        });
        if (disposed) { stop(); return; }
        unlisten = stop;
        const items = await call<AutomationStatus[]>('list_automation');
        if (!disposed) setItems(items);
      } catch (e) { if (!disposed) setError(String(e)); }
      finally { if (!disposed) setLoading(false); }
    })();
    return () => { disposed = true; mounted.current = false; unlisten?.(); };
  }, []);
  async function action(command: string, kind: AutomationKind, args: Record<string, unknown> = {}) {
    setWorking(true); setError('');
    try { const result = await call<AutomationStatus[]>(command, { kind, ...args }); if (mounted.current) setItems(result); }
    catch (e) {
      if (mounted.current) setError(String(e));
      try { const result = await call<AutomationStatus[]>('list_automation'); if (mounted.current) setItems(result); } catch { /* Keep the error and previous state. */ }
    } finally { if (mounted.current) setWorking(false); }
  }
  const locked = busy || working || loading || items.some(item => automationWorking(item.phase));
  return <div className="client-settings-page automation-page">
    <div className="settings-page-heading"><h2>自动化工具</h2></div>
    <p className="automation-intro">让 Agent 操作网页和本机应用。安装并测试后即可使用。</p>
    {!desktop ? <p className="automation-notice">请在 SuperCode 桌面端安装和测试。</p> : busy ? <p className="automation-notice">当前任务结束后，可以安装或更改工具。</p> : null}
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    <div className="automation-cards" aria-busy={loading}>
      {items.map(item => {
        const browser = item.kind === 'browser'; const running = automationWorking(item.phase);
        const failed = item.phase === 'failed' || item.phase === 'missing';
        const ready = item.phase === 'ready' && !!item.installation;
        const size = automationDownload(item);
        return <section className="automation-card" key={item.kind} aria-label={browser ? '浏览器自动化' : '电脑自动化'}>
          <header><div className="automation-icon">{browser ? <Globe size={22}/> : <Monitor size={22}/>}</div><div className="automation-title"><h3>{browser ? '浏览器自动化' : '电脑自动化'}</h3><p>{browser ? 'Playwright · 独立 Chromium 浏览器' : 'Windows MCP · 本机应用'}</p></div>
            {ready ? <label className="automation-enable"><span>{item.enabled ? '已启用' : '已停用'}</span><input className="settings-switch" role="switch" type="checkbox" aria-label={`启用${browser ? '浏览器' : '电脑'}自动化`} checked={item.enabled} disabled={locked || !desktop} onChange={e => void action('set_automation_enabled', item.kind, { enabled: e.target.checked })}/></label> : null}
          </header>
          <p className="automation-description">{browser ? '浏览网页、点击、填写表单和截图。使用独立会话，保留你的日常浏览器环境。' : '读取桌面、操作窗口、点击和输入。工具随任务启动。'}</p>
          <div className={`automation-result ${failed ? 'failed' : ''}`} role="status">
            {loading ? <span>正在检查安装状态…</span> : running ? <><RefreshCw size={14} className="automation-spin"/><span>{item.message}</span>{size ? <small>{size}</small> : null}</> : ready ? <><Check size={15}/><span>测试通过</span><small>{item.installation!.tools} 个工具 · v{item.installation!.version}</small></> : <span>{item.message}</span>}
          </div>
          {running ? <progress aria-label={`${browser ? '浏览器' : '电脑'}自动化安装进度`} max={item.total ?? undefined} value={item.total && item.received != null ? item.received : undefined}/> : null}
          {item.installation ? <div className="automation-test-summary"><p>{item.installation.summary}</p><span>上次通过测试 {new Date(item.installation.testedAt * 1000).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}</span></div> : null}
          <footer>{running ? <button className="quiet-button" onClick={() => { void call('cancel_automation', { kind: item.kind }).catch(e => setError(String(e))); }}><X size={14}/>取消</button> : ready ? <><button className="quiet-button" disabled={locked || !desktop} onClick={() => void action('test_automation', item.kind)}><RefreshCw size={14}/>重新测试</button><button className="automation-text-button" disabled={locked || !desktop} onClick={() => void action('install_automation', item.kind, { repair: true })}>修复安装</button></> : <button className="primary-button" disabled={locked || !desktop} onClick={() => void action('install_automation', item.kind, { repair: item.phase === 'missing' })}><Download size={14}/>{failed ? '重试安装并测试' : '安装并测试'}</button>}</footer>
        </section>;
      })}
    </div>
    <div className="automation-footnote"><ShieldCheck size={16}/><p>安装到 SuperCode 的独立目录，Codex、Claude Code、OpenCode 和 Pi 均可使用。测试结束后释放进程，工具在任务中按需启动。<br/>自定义 MCP 连接可在“插件与 MCP”中管理。</p></div>
  </div>;
}
