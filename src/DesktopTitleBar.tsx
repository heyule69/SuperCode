import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';
import { ArrowLeft, ArrowRight, Check, Copy, Minus, PanelLeft, Square, X } from 'lucide-react';
import { desktop } from './api';
export interface DesktopMenuItem { label: string; shortcut?: string; run: () => void; disabled?: boolean; checked?: boolean; divider?: boolean }
export interface DesktopMenu { label: string; items: DesktopMenuItem[] }
export function DesktopTitleBar({ back, forward, canBack, canForward, sidebar, toggleSidebar, menus, blocked, report }: {
  back: () => void; forward: () => void; canBack: boolean; canForward: boolean; sidebar: boolean; toggleSidebar: () => void;
  menus: DesktopMenu[]; blocked: boolean; report: (error: unknown) => void;
}) {
  const [menu, setMenu] = useState<number | null>(null);
  const [maximized, setMaximized] = useState(false);
  const bar = useRef<HTMLElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const origin = useRef<HTMLElement | null>(null);
  const layerId = useId();
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      const update = async () => { const value = await win.isMaximized(); if (!disposed) setMaximized(value); };
      await update();
      const off = await win.onResized(() => { void update().catch(report); });
      if (disposed) off(); else unlisten = off;
    }).catch(report);
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => { if (blocked) setMenu(null); }, [blocked]);
  useEffect(() => {
    if (menu === null) return;
    document.dispatchEvent(new CustomEvent('supercode:layer-open', { detail: layerId }));
    popup.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    const outside = (event: Event) => { if (!bar.current?.contains(event.target as Node)) setMenu(null); };
    const other = (event: Event) => { if ((event as CustomEvent).detail !== layerId) setMenu(null); };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('supercode:layer-open', other);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('supercode:layer-open', other); };
  }, [menu, layerId]);
  function close(restore = true) { setMenu(null); if (restore) origin.current?.focus({ preventScroll: true }); }
  function navigate(event: KeyboardEvent) {
    if (menu === null) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
    else if (event.key === 'Tab') close(false);
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setMenu((menu + (event.key === 'ArrowRight' ? 1 : menus.length - 1)) % menus.length); }
    else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault(); const items = [...popup.current!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
      const index = items.indexOf(document.activeElement as HTMLButtonElement);
      items[event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();
    }
  }
  async function windowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
    try { const { getCurrentWindow } = await import('@tauri-apps/api/window'); await getCurrentWindow()[action](); } catch (error) { report(error); }
  }
  return <header className="desktop-titlebar" ref={bar} onKeyDown={navigate} aria-label="应用菜单">
    <div className="desktop-navigation"><button title="后退 (Alt+←)" aria-label="后退" disabled={blocked || !canBack} onClick={back}><ArrowLeft size={16}/></button><button title="前进 (Alt+→)" aria-label="前进" disabled={blocked || !canForward} onClick={forward}><ArrowRight size={16}/></button><button title="切换侧栏 (Ctrl+B)" aria-label="切换侧栏" aria-pressed={sidebar} disabled={blocked} onClick={toggleSidebar}><PanelLeft size={16}/></button></div>
    <nav className="desktop-menubar" aria-label="主菜单">{menus.map((group, index) => <div className="desktop-menu" key={group.label}><button className={menu === index ? 'selected' : ''} aria-haspopup="menu" aria-expanded={menu === index} disabled={blocked} onMouseDown={event => event.preventDefault()} onClick={event => { origin.current = event.currentTarget; setMenu(menu === index ? null : index); }} onMouseEnter={() => { if (menu !== null) setMenu(index); }}>{group.label}</button>{menu === index ? <div className="desktop-menu-popup" role="menu" aria-label={group.label} ref={popup}>{group.items.map(item => <button key={item.label} className={item.divider ? 'menu-divider' : ''} role={item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'} aria-checked={item.checked} disabled={item.disabled} onClick={() => { close(false); item.run(); }}><span className="desktop-menu-check">{item.checked ? <Check size={13}/> : null}</span><span>{item.label}</span>{item.shortcut ? <small>{item.shortcut}</small> : null}</button>)}</div> : null}</div>)}</nav>
    <div className="desktop-drag-region" data-tauri-drag-region/>
    {desktop ? <div className="desktop-window-controls"><button aria-label="最小化" title="最小化" onClick={() => void windowAction('minimize')}><Minus size={15}/></button><button aria-label={maximized ? '还原窗口' : '最大化'} title={maximized ? '还原窗口' : '最大化'} onClick={() => void windowAction('toggleMaximize')}>{maximized ? <Copy size={13}/> : <Square size={13}/>}</button><button className="window-close" aria-label="关闭窗口" title="最小化到系统托盘" onClick={() => void windowAction('close')}><X size={17}/></button></div> : null}
  </header>;
}
