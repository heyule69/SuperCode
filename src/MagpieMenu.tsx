import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, Download, ExternalLink, Info, LoaderCircle, PanelTop, Plus } from 'lucide-react';
import { ProviderIcon } from './AgentIcon';
import { call, desktop } from './api';

const downloadUrl = 'https://github.com/yetone/magpie-releases/releases/latest';
export function MagpieMenu({ added, count, busy, run, use }: { added: boolean; count: number; busy: boolean; run: (action: () => Promise<void>) => Promise<void>; use: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null), menu = useRef<HTMLDivElement>(null);
  function close(focus = false) { setOpen(false); if (focus) trigger.current?.focus({ preventScroll: true }); }
  useEffect(() => { if (busy) setOpen(false); }, [busy]);
  useEffect(() => {
    if (!open) return;
    const outside = (e: Event) => { if (!menu.current?.contains(e.target as Node) && !trigger.current?.contains(e.target as Node)) close(); };
    document.addEventListener('pointerdown', outside); document.addEventListener('focusin', outside);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); };
  }, [open]);
  useLayoutEffect(() => {
    if (!open) return;
    const host = trigger.current?.closest<HTMLElement>('.provider-settings')?.parentElement;
    function position() {
      const popup = menu.current, button = trigger.current?.getBoundingClientRect(), boundary = host?.getBoundingClientRect();
      if (!popup || !button) return;
      const left = Math.max(12, boundary?.left ?? 12), right = Math.min(innerWidth - 12, boundary?.right ?? innerWidth - 12);
      popup.style.maxWidth = `${Math.max(0, right - left)}px`;
      popup.style.maxHeight = `${Math.max(100, innerHeight - 24)}px`;
      const rect = popup.getBoundingClientRect(), below = innerHeight - button.bottom - 20, above = button.top - 20;
      popup.style.maxHeight = `${Math.max(100, Math.max(below, above))}px`;
      const height = popup.getBoundingClientRect().height;
      popup.style.left = `${Math.max(left, Math.min(button.right - rect.width, right - rect.width))}px`;
      popup.style.top = `${below >= height || below >= above ? button.bottom + 8 : Math.max(12, button.top - height - 8)}px`;
      popup.style.visibility = 'visible';
    }
    position(); menu.current?.querySelector<HTMLAnchorElement>('a')?.focus({ preventScroll: true });
    window.addEventListener('resize', position); document.addEventListener('scroll', position, true);
    const observer = new ResizeObserver(position); if (host) observer.observe(host);
    return () => { window.removeEventListener('resize', position); document.removeEventListener('scroll', position, true); observer.disconnect(); };
  }, [open]);
  function navigate(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); close(true); return; }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) return;
    e.preventDefault();
    const items = [...menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? []];
    const index = items.indexOf(document.activeElement as HTMLElement);
    items[e.key === 'Home' ? 0 : e.key === 'End' ? items.length - 1 : (index + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus({ preventScroll: true });
  }
  async function action(callback: () => Promise<void>) { close(true); await run(callback); }
  return <>
    <button ref={trigger} className="quiet-button magpie-trigger" aria-label="Magpie" aria-expanded={open} aria-haspopup="menu" disabled={busy} onClick={() => setOpen(value => !value)}><ProviderIcon provider="magpie"/><span>Magpie</span><span className={`magpie-status-dot ${added ? 'is-added' : ''}`} aria-hidden="true"/><ChevronDown size={12}/></button>
    {open ? createPortal(<div ref={menu} className="magpie-popover" role="menu" aria-label="Magpie 操作" style={{ visibility: 'hidden' }} onKeyDown={navigate}>
      <div className="magpie-menu-heading"><span className="provider-mark"><ProviderIcon provider="magpie"/></span><div><strong>Magpie</strong><small>{added ? `已添加 · ${count} 个模型可用` : '尚未添加到 SuperCode'}</small></div></div>
      <a href={downloadUrl} target="_blank" rel="noreferrer" role="menuitem" onClick={e => { if (desktop) { e.preventDefault(); void action(async () => { await call('open_external_link', { url: downloadUrl }); }); } else close(true); }}><Download size={18}/><span><strong>下载 Magpie</strong><small>前往官方下载页面</small></span><ExternalLink size={14}/></a>
      <button role="menuitem" disabled={!desktop} onClick={() => void action(async () => { await call('open_magpie'); })}><PanelTop size={18}/><span><strong>打开 Magpie</strong><small>在 Magpie 中管理供应商与账号</small></span><ExternalLink size={14}/></button>
      <button role="menuitem" className="magpie-use" disabled={!desktop || added} onClick={() => void action(use)}>{added ? <Check size={18}/> : busy ? <LoaderCircle size={18} className="spin"/> : <Plus size={18}/>}<span><strong>{added ? '已添加到 SuperCode' : '使用 Magpie'}</strong><small>{added ? '无需填写或编辑 API 配置' : '自动添加连接，同步全部模型'}</small></span></button>
      <p className="magpie-menu-note"><Info size={12}/>使用期间，保持 Magpie 运行即可。</p>
    </div>, document.body) : null}
  </>;
}
