import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { ChevronDown, ChevronRight, ExternalLink, LoaderCircle, Monitor, Plus, Power, SquarePen } from 'lucide-react';
import { menuKeyIndex, trayStatus, type TraySnapshot } from './trayMenuModel';

export function TrayMenu({ data, error, perform, ready }: {
  data?: TraySnapshot; error: string; perform: (action: string, value?: string) => Promise<void>; ready: (height: number) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [windowsOpen, setWindowsOpen] = useState(false);
  const [working, setWorking] = useState(false);
  const panel = useRef<HTMLElement>(null);
  const token = data?.token;
  useLayoutEffect(() => { setExpanded(false); setWindowsOpen(false); setWorking(false); }, [token]);
  useLayoutEffect(() => {
    if (!data) return;
    const frame = requestAnimationFrame(() => {
      const element = panel.current;
      if (!element) return;
      let height = Math.max(element.getBoundingClientRect().height, element.scrollHeight + 2);
      for (const child of element.querySelectorAll<HTMLElement>('.tray-recent-list, .tray-window-list')) {
        const style = getComputedStyle(child);
        const maximum = parseFloat(style.maxHeight);
        height += Math.max(0, Math.min(child.scrollHeight, Number.isFinite(maximum) ? maximum : child.scrollHeight) - child.clientHeight);
      }
      ready(height);
    });
    return () => cancelAnimationFrame(frame);
  }, [data, expanded, windowsOpen, error, ready]);
  async function run(action: string, value?: string) {
    if (working) return;
    setWorking(true);
    try { await perform(action, value); } finally { setWorking(false); }
  }
  function navigate(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); void run('dismiss'); return; }
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = [...panel.current!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    items[menuKeyIndex(event.key, items.indexOf(document.activeElement as HTMLButtonElement), items.length)]?.focus();
  }
  function status(value: string, unread = false) {
    const kind = trayStatus(value, unread);
    return <span className={`tray-status ${kind}`} aria-label={kind === 'running' ? '正在运行' : kind === 'waiting' ? '等待确认' : kind === 'unread' ? '未读' : undefined}>
      {kind === 'running' ? <LoaderCircle size={13}/> : kind === 'waiting' ? <span className="tray-waiting">!</span> : kind === 'unread' ? <span className="tray-unread"/> : null}
    </span>;
  }
  function action(label: string, name: string, icon: ReactNode) {
    return <button role="menuitem" className="tray-action" disabled={working} onClick={() => void run(name)}>{icon}<span>{label}</span></button>;
  }
  return <main className="tray-panel" ref={panel} role="menu" aria-label="SuperCode" onKeyDown={navigate}>
    <div className="tray-heading">最近对话</div>
    {!data ? <p className="tray-empty">{error || '正在读取对话…'}</p> : <>
      {data.recent.length ? <div className="tray-recent-list">{data.recent.slice(0, expanded ? 30 : 3).map(chat =>
        <button className="tray-chat" role="menuitem" key={chat.id} disabled={working} title={`${chat.title} · ${chat.project}`} onClick={() => void run('session', chat.id)}>
          {status(chat.status, chat.unread)}<span className="tray-title">{chat.title}</span><span className="tray-project">{chat.project}</span>
        </button>)}
      </div> : <p className="tray-empty">还没有对话，开始一个新对话吧</p>}
      {data.recent.length > 3 ? <button className="tray-more" role="menuitem" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}><span>{expanded ? '收起' : '更多对话'}</span>{expanded ? <ChevronDown size={15}/> : <ChevronRight size={15}/>}</button> : null}
      {expanded && data.hasMore ? <button className="tray-more" role="menuitem" onClick={() => void run('search')}><span>搜索全部对话</span><ExternalLink size={14}/></button> : null}
      {data.windows.length > 1 ? <>
        <div className="tray-divider" role="separator"/>
        <button className="tray-more tray-window-toggle" role="menuitem" aria-expanded={windowsOpen} onPointerEnter={() => setWindowsOpen(true)} onClick={() => setWindowsOpen(value => !value)}><span>打开的窗口 <small>{data.windows.length}</small></span>{windowsOpen ? <ChevronDown size={15}/> : <ChevronRight size={15}/>}</button>
        {windowsOpen ? <div className="tray-window-list">{data.windows.map(win => <button className="tray-chat" role="menuitem" key={win.label} disabled={working} title={`${win.title} · ${win.project}`} onClick={() => void run('window', win.label)}>{status(win.status)}<span className="tray-title">{win.title}</span><span className="tray-project">{win.main ? '主窗口' : win.project || '新窗口'}</span></button>)}</div> : null}
      </> : null}
    </>}
    {data && error ? <p className="tray-error" role="alert">{error}</p> : null}
    <div className="tray-divider" role="separator"/>
    {action('新对话', 'new-chat', <SquarePen size={16}/>)}
    {action('新窗口', 'new-window', <Plus size={17}/>)}
    {action('显示主窗口', 'show', <Monitor size={16}/>)}
    <div className="tray-divider" role="separator"/>
    {action('退出', 'quit', <Power size={16}/>)}
  </main>;
}
