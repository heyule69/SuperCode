import { useEffect, useId, useLayoutEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Copy, ExternalLink, FileText, FolderOpen } from 'lucide-react';
import { call, desktop } from './api';
import { describeLink, resourceFromText, systemDocument } from './links';

type Resource = { kind: 'file' | 'external'; target: string; line?: number };
export type OpenPath = (path: string, action: 'open' | 'reveal') => void | Promise<void>;

export function ResourceMenu({ children, openFile, openPath }: { children: ReactNode; openFile: (path: string, line?: number) => void; openPath?: OpenPath }) {
  const [menu, setMenu] = useState<{ resource: Resource; x: number; y: number; origin: HTMLElement } | null>(null);
  const [feedback, setFeedback] = useState('');
  const [busy, setBusy] = useState(false);
  const popup = useRef<HTMLDivElement>(null);
  const id = useId();
  const close = (restore = false) => { if (restore) menu?.origin.focus({ preventScroll: true }); setMenu(null); };
  useEffect(() => {
    if (!menu) return;
    document.dispatchEvent(new CustomEvent('supercode:layer-open', { detail: id }));
    const outside = (event: Event) => { if (!popup.current?.contains(event.target as Node)) setMenu(null); };
    const another = (event: Event) => { if ((event as CustomEvent).detail !== id) setMenu(null); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); menu.origin.focus({ preventScroll: true }); setMenu(null); } };
    const scroll = () => setMenu(null);
    document.addEventListener('pointerdown', outside);
    document.addEventListener('focusin', outside);
    document.addEventListener('keydown', escape, true);
    document.addEventListener('supercode:layer-open', another);
    window.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', scroll);
    return () => {
      document.removeEventListener('pointerdown', outside);
      document.removeEventListener('focusin', outside);
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('supercode:layer-open', another);
      window.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', scroll);
    };
  }, [menu, id]);
  useLayoutEffect(() => {
    if (!menu || !popup.current) return;
    const bounds = popup.current.getBoundingClientRect();
    popup.current.style.left = `${Math.max(8, Math.min(menu.x, window.innerWidth - bounds.width - 8))}px`;
    popup.current.style.top = `${Math.max(8, Math.min(menu.y, window.innerHeight - bounds.height - 8))}px`;
    popup.current.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
  }, [menu]);

  function show(event: MouseEvent<HTMLDivElement>) {
    const target = event.target as HTMLElement;
    const tagged = target.closest<HTMLElement>('[data-resource-path]');
    const anchor = target.closest<HTMLAnchorElement>('a[href]');
    const code = target.closest<HTMLElement>('code:not(pre code)');
    const selection = window.getSelection();
    const selected = selection && !selection.isCollapsed && event.currentTarget.contains(selection.anchorNode) ? resourceFromText(selection.toString()) : null;
    const resource = tagged ? describeLink(tagged.dataset.resourcePath ?? '') : anchor ? describeLink(anchor.getAttribute('href') ?? '') : selected ?? (code ? resourceFromText(code.textContent ?? '') : null);
    if (!resource || (resource.kind !== 'file' && resource.kind !== 'external')) return;
    event.preventDefault();
    const origin = tagged ?? anchor ?? code ?? target;
    const bounds = origin.getBoundingClientRect();
    setFeedback(''); setBusy(false);
    setMenu({ resource: { kind: resource.kind, target: resource.target, line: resource.line }, x: event.clientX || bounds.left, y: event.clientY || bounds.bottom, origin });
  }
  async function act(action: 'open' | 'reveal' | 'copy') {
    if (!menu || busy) return;
    setFeedback(''); setBusy(true);
    try {
      const resource = menu.resource;
      if (action === 'copy') { await navigator.clipboard.writeText(resource.target); setFeedback('已复制'); }
      else {
        if (resource.kind === 'external') {
          if (desktop) await call('open_external_link', { url: resource.target });
          else window.open(resource.target, '_blank', 'noopener,noreferrer');
        } else if (action === 'reveal' || systemDocument(resource.target)) {
          if (!openPath) throw new Error('请在桌面版中打开文件');
          await openPath(resource.target, action);
        } else openFile(resource.target, resource.line);
        close();
      }
    } catch (error) { setFeedback(String(error)); }
    finally { setBusy(false); }
  }
  return <div className="resource-menu-host" onContextMenu={show}>{children}{menu ? createPortal(<div ref={popup} className="resource-context-menu" role="menu" aria-label={menu.resource.kind === 'file' ? '文件操作' : '链接操作'} style={{ left: menu.x, top: menu.y }} onKeyDown={event => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End', 'Tab'].includes(event.key)) return;
    if (event.key === 'Tab') { close(true); return; }
    const buttons = [...popup.current!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    if (!buttons.length) return;
    event.preventDefault();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowDown' ? (at + 1) % buttons.length : (at + buttons.length - 1) % buttons.length;
    buttons[next].focus();
  }}>
    <button role="menuitem" disabled={busy} onClick={() => void act('open')}>{menu.resource.kind === 'file' ? <FileText size={15}/> : <ExternalLink size={15}/>}<span>{menu.resource.kind === 'file' ? '打开文件' : '打开链接'}</span></button>
    {menu.resource.kind === 'file' ? <button role="menuitem" disabled={busy || !openPath} onClick={() => void act('reveal')}><FolderOpen size={15}/><span>在文件夹中显示</span></button> : null}
    <button role="menuitem" disabled={busy} onClick={() => void act('copy')}><Copy size={15}/><span>{feedback === '已复制' ? feedback : menu.resource.kind === 'file' ? '复制路径' : '复制链接'}</span></button>
    {feedback && feedback !== '已复制' ? <div className="resource-menu-error" role="alert">{feedback}</div> : null}
  </div>, document.body) : null}</div>;
}
