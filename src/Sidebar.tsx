import { useEffect, useId, useLayoutEffect, useRef, useState, type Dispatch, type MouseEvent, type SetStateAction } from 'react';
import { createPortal } from 'react-dom';
import { Archive, Check, ChevronRight, Copy, Download, ExternalLink, Eye, EyeOff, Folder, FolderClosed, FolderOpen, FolderInput, GitFork, List, LoaderCircle, MoreHorizontal, PanelTop, Pencil, Pin, PinOff, Plus, Settings2, Share2, SquarePen, Terminal, Trash2, X, type LucideIcon } from 'lucide-react';
import { call, desktop } from './api';
import { exportFilename, projectSessions, sidebarAreas, type SidebarGroup } from './sidebarState';
import { isActive, type ArchivedSession, type Project, type Session, type SidebarSection, type SidebarState } from './types';
import { SessionIndicator } from './SessionIndicator';

type Target = { kind: 'project'; item: Project } | { kind: 'session'; item: Session } | { kind: 'section'; item: SidebarSection } | { kind: 'root' };
type SessionDialog = { [Kind in 'rename' | 'share' | 'delete']: { kind: Kind; session: Session } }['rename' | 'share' | 'delete'];
type ProjectDialog = { [Kind in 'edit' | 'remove' | 'archiveProject']: { kind: Kind; project: Project } }['edit' | 'remove' | 'archiveProject'];
type Dialog = SessionDialog | ProjectDialog | { kind: 'section'; section?: SidebarSection; target?: Target } | { kind: 'archives' };
interface MenuItem { id?: string; label: string; description?: string; icon?: LucideIcon; shortcut?: string; run?: () => void; items?: MenuItem[]; disabled?: boolean; checked?: boolean; danger?: boolean; divider?: boolean }
export interface SidebarProps {
  projects: Project[]; sessions: Session[]; state: SidebarState; projectId: string; sessionId: string; ready: boolean; blocked: boolean;
  collapsed: Set<string>; setCollapsed: Dispatch<SetStateAction<Set<string>>>;
  selectProject: (project: Project) => void; selectSession: (session: Session) => void | Promise<void>;
  addProject: () => void; createSession: (project?: Project) => Promise<void>; updated: () => Promise<void>; dialogChanged: (open: boolean) => void;
}

export function Sidebar(props: SidebarProps) {
  const { projects, sessions, state, projectId, sessionId, ready, blocked, collapsed, setCollapsed, selectProject, selectSession, addProject, createSession, updated, dialogChanged } = props;
  const [menu, setMenu] = useState<{ target: Target; x: number; y: number; origin: HTMLElement } | null>(null);
  const [sub, setSub] = useState<{ parent: string; items: MenuItem[]; x: number; y: number; focus: boolean } | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [title, setTitle] = useState('');
  const [path, setPath] = useState('');
  const [error, setError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [working, setWorking] = useState(false);
  const [archives, setArchives] = useState<ArchivedSession[]>([]);
  const [archiveSearch, setArchiveSearch] = useState('');
  const [archiveLoading, setArchiveLoading] = useState(false);
  const popup = useRef<HTMLDivElement>(null);
  const panel = useRef<HTMLElement>(null);
  const origin = useRef<HTMLElement | null>(null);
  const current = useRef(props); current.current = props;
  const actionLock = useRef(false);
  const layerId = useId();

  function closeMenu(restore = false) { if (restore) menu?.origin.focus({ preventScroll: true }); setMenu(null); setSub(null); setError(''); setFeedback(''); }
  function show(event: MouseEvent, target: Target, fromButton = false) {
    if (blocked || dialog || working) return;
    event.preventDefault(); event.stopPropagation();
    const origin = event.currentTarget as HTMLElement;
    const bounds = origin.getBoundingClientRect();
    setError(''); setFeedback(''); setSub(null);
    setMenu({ target, x: fromButton ? bounds.right : event.clientX || bounds.left, y: fromButton ? bounds.bottom : event.clientY || bounds.bottom, origin });
  }
  function openDialog(next: Dialog) {
    origin.current = menu?.origin ?? document.activeElement as HTMLElement;
    closeMenu(); setTitle(next.kind === 'rename' ? next.session.title : next.kind === 'edit' ? next.project.name : next.kind === 'section' ? next.section?.name ?? '' : '');
    setPath(next.kind === 'edit' ? next.project.path : ''); setError(''); setFeedback(''); setDialog(next);
  }
  function closeDialog() { if (actionLock.current) return; setDialog(null); setError(''); setFeedback(''); }
  useEffect(() => { dialogChanged(!!dialog); return () => dialogChanged(false); }, [dialog, dialogChanged]);
  useEffect(() => {
    if (!menu) return;
    document.dispatchEvent(new CustomEvent('supercode:layer-open', { detail: layerId }));
    const outside = (e: Event) => { if (!popup.current?.contains(e.target as Node)) { setMenu(null); setSub(null); } };
    const another = (e: Event) => { if ((e as CustomEvent).detail !== layerId) { setMenu(null); setSub(null); } };
    const escape = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); if (sub) { setSub(null); popup.current?.querySelector<HTMLButtonElement>('[data-submenu="'+sub.parent+'"]')?.focus(); } else closeMenu(true); } };
    const scroll = (e: Event) => { if (popup.current?.contains(e.target as Node)) return; setMenu(null); setSub(null); };
    document.addEventListener('pointerdown', outside); document.addEventListener('focusin', outside); document.addEventListener('keydown', escape, true);
    document.addEventListener('supercode:layer-open', another); window.addEventListener('resize', scroll); window.addEventListener('scroll', scroll, true);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); document.removeEventListener('keydown', escape, true); document.removeEventListener('supercode:layer-open', another); window.removeEventListener('resize', scroll); window.removeEventListener('scroll', scroll, true); };
  }, [menu, sub, layerId]);
  useLayoutEffect(() => {
    if (!menu || !popup.current) return;
    const bounds = popup.current.getBoundingClientRect();
    popup.current.style.left = `${Math.max(8, Math.min(menu.x, window.innerWidth - bounds.width - 8))}px`;
    popup.current.style.top = `${Math.max(8, Math.min(menu.y, window.innerHeight - bounds.height - 8))}px`;
    popup.current.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
  }, [menu]);
  useLayoutEffect(() => {
    const element = popup.current?.querySelector<HTMLElement>('.sidebar-submenu'); if (!sub || !element) return;
    const bounds = element.getBoundingClientRect();
    element.style.left = `${Math.max(8, Math.min(sub.x, window.innerWidth - bounds.width - 8))}px`;
    element.style.top = `${Math.max(8, Math.min(sub.y, window.innerHeight - bounds.height - 8))}px`;
    if (sub.focus) element.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
  }, [sub]);
  useEffect(() => {
    if (!dialog) return;
    const previous = origin.current;
    (panel.current?.querySelector<HTMLInputElement>('input') ?? panel.current?.querySelector<HTMLButtonElement>('button:not(:disabled)'))?.focus();
    const trap = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !actionLock.current) { e.preventDefault(); e.stopImmediatePropagation(); setDialog(null); }
      if (e.key !== 'Tab' || !panel.current) return;
      const items = [...panel.current.querySelectorAll<HTMLElement>('input,button:not(:disabled)')].filter(el => el.offsetParent !== null);
      if (e.shiftKey && document.activeElement === items[0]) { e.preventDefault(); items.at(-1)?.focus(); }
      else if (!e.shiftKey && document.activeElement === items.at(-1)) { e.preventDefault(); items[0]?.focus(); }
    };
    document.addEventListener('keydown', trap, true);
    return () => { document.removeEventListener('keydown', trap, true); if (previous?.isConnected) previous.focus({ preventScroll: true }); };
  }, [dialog]);
  useEffect(() => {
    function shortcuts(event: KeyboardEvent) {
      if (current.current.blocked || dialog || menu || actionLock.current || !event.ctrlKey || event.metaKey) return;
      const session = current.current.sessions.find(s => s.id === current.current.sessionId); if (!session) return;
      const key = event.key.toLowerCase();
      if (event.altKey && key === 'r') { event.preventDefault(); openDialog({ kind: 'rename', session }); }
      else if (event.altKey && key === 'p') { event.preventDefault(); void update('session', session.id, 'pin', !current.current.state.sessions[session.id]?.pinned); }
      else if (event.shiftKey && key === 'u') { event.preventDefault(); void update('session', session.id, 'unread', !current.current.state.sessions[session.id]?.unread); }
      else if (event.shiftKey && key === 'a' && !isActive(session.status)) { event.preventDefault(); void command('archive_session', { sessionId: session.id }); }
    }
    window.addEventListener('keydown', shortcuts); return () => window.removeEventListener('keydown', shortcuts);
  }, [dialog, menu]);

  async function run(action: () => Promise<void>, close = true) {
    if (actionLock.current) return;
    actionLock.current = true; setWorking(true); setError(''); setFeedback('');
    try { await action(); if (close) { setMenu(null); setSub(null); setDialog(null); } }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { actionLock.current = false; setWorking(false); }
  }
  async function command(name: string, args: Record<string, unknown>) { await run(async () => { await call(name, args); await updated(); }); }
  async function update(kind: 'project' | 'session', id: string, action: string, value: unknown) { await command('update_sidebar_item', { kind, id, action, value }); }
  async function copy(text: string) { await run(async () => { await navigator.clipboard.writeText(text); setFeedback('已复制'); }, false); }
  async function fork(session: Session, projectId?: string) { await run(async () => { const next = await call<Session>('fork_sidebar_session', { sessionId: session.id, projectId: projectId ?? null }); await updated(); await selectSession(next); }); }
  async function loadArchives() {
    setArchiveLoading(true);
    try { setArchives(await call<ArchivedSession[]>('list_archived_sessions')); } catch (e) { setError(String(e)); } finally { setArchiveLoading(false); }
  }
  useEffect(() => { if (dialog?.kind === 'archives') { setArchiveSearch(''); void loadArchives(); } }, [dialog]);

  function sectionItems(target: Target): MenuItem[] {
    if (target.kind !== 'project' && target.kind !== 'session') return [];
    const item = state[target.kind === 'project' ? 'projects' : 'sessions'][target.item.id];
    return [
      { label: '默认', checked: !item?.sectionId, run: () => void update(target.kind, target.item.id, 'section', null) },
      ...state.sections.map(s => ({ id: s.id, label: s.name, checked: item?.sectionId === s.id, run: () => void update(target.kind, target.item.id, 'section', s.id) })),
      { label: '新建分区…', icon: Plus, divider: true, run: () => openDialog({ kind: 'section', target }) },
    ];
  }
  function menuItems(target: Target): MenuItem[] {
    if (target.kind === 'root') return [{ label: '新建分区…', icon: Plus, run: () => openDialog({ kind: 'section' }) }, { label: '已归档聊天', icon: Archive, run: () => openDialog({ kind: 'archives' }) }];
    if (target.kind === 'section') return [{ label: '重命名分区', icon: Pencil, run: () => openDialog({ kind: 'section', section: target.item }) }, { label: '移除分区', icon: X, run: () => void command('delete_sidebar_section', { id: target.item.id }) }];
    const { item } = target;
    const meta = state[target.kind === 'project' ? 'projects' : 'sessions'][item.id];
    if (target.kind === 'project') {
      const busy = sessions.some(s => s.projectId === item.id && isActive(s.status));
      return [
        { label: meta?.pinned ? '取消置顶' : '置顶', icon: meta?.pinned ? PinOff : Pin, run: () => void update('project', item.id, 'pin', !meta?.pinned) },
        { label: '编辑项目', icon: Settings2, disabled: busy, run: () => openDialog({ kind: 'edit', project: target.item }) },
        { label: '分区', icon: List, divider: true, items: sectionItems(target) },
        { label: '在资源管理器中打开', icon: FolderOpen, disabled: !desktop, run: () => void command('open_sidebar_project', { projectId: item.id, mode: 'folder' }) },
        { label: '打开方式', icon: ExternalLink, items: [{ label: 'VS Code', icon: PanelTop, disabled: !desktop, run: () => void command('open_sidebar_project', { projectId: item.id, mode: 'vscode' }) }, { label: '系统终端', icon: Terminal, disabled: !desktop, run: () => void command('open_sidebar_project', { projectId: item.id, mode: 'terminal' }) }] },
        { label: '复制路径', icon: Copy, run: () => void copy(target.item.path) },
        { label: '归档聊天', icon: Archive, divider: true, disabled: busy || !sessions.some(s => s.projectId === item.id), run: () => openDialog({ kind: 'archiveProject', project: target.item }) },
        { label: '移除项目', icon: X, divider: true, disabled: busy, run: () => openDialog({ kind: 'remove', project: target.item }) },
      ];
    }
    const session = target.item; const busy = isActive(session.status);
    return [
      { label: '重命名', icon: Pencil, shortcut: 'Alt+Ctrl+R', run: () => openDialog({ kind: 'rename', session }) },
      { label: meta?.pinned ? '取消置顶' : '置顶', icon: meta?.pinned ? PinOff : Pin, shortcut: 'Alt+Ctrl+P', run: () => void update('session', session.id, 'pin', !meta?.pinned) },
      { label: meta?.unread ? '标记为已读' : '标记为未读', icon: meta?.unread ? EyeOff : Eye, shortcut: 'Ctrl+Shift+U', run: () => void update('session', session.id, 'unread', !meta?.unread) },
      { label: '项目', icon: FolderInput, disabled: busy, items: projects.map(p => ({ id:p.id, label: p.name, description: projects.some(other=>other.id!==p.id && other.name===p.name) ? p.path : undefined, checked: p.id === session.projectId, run: () => void command('move_sidebar_session', { sessionId: session.id, projectId: p.id }) })) },
      { label: '分区', icon: List, items: sectionItems(target) },
      { label: '分叉', icon: GitFork, divider: true, disabled: busy, items: [{ label: session.projectId ? '在当前项目分叉' : '在当前聊天目录分叉', icon: GitFork, run: () => void fork(session) }, ...projects.filter(p => p.id !== session.projectId).map(p => ({ id:p.id, label: p.name, description: projects.some(other=>other.id!==p.id && other.name===p.name) ? p.path : undefined, icon: Folder, run: () => void fork(session, p.id) }))] },
      { label: '分享', icon: Share2, divider: true, run: () => openDialog({ kind: 'share', session }) },
      { label: '复制', icon: Copy, items: [{ label: '聊天标题', run: () => void copy(session.title) }, { label: '聊天内容', run: () => void run(async () => { const text = await call<string>('copy_chat_transcript', { sessionId: session.id }); await navigator.clipboard.writeText(text); setFeedback('已复制'); }, false) }, { label: '聊天 ID', run: () => void copy(session.id) }] },
      { label: '在新窗口中打开', icon: PanelTop, divider: true, disabled: !desktop, run: () => void command('open_chat_window', { sessionId: session.id, dark: document.documentElement.dataset.theme === 'dark' || document.documentElement.dataset.theme === 'graphite' }) },
      { label: '打开方式', icon: ExternalLink, items: [{ label: '在当前窗口中打开', icon: PanelTop, run: () => { closeMenu(); void selectSession(session); } }, { label: '导出 Markdown…', icon: Download, disabled: !desktop, run: () => void exportChat(session, 'md') }] },
      { label: '归档', icon: Archive, shortcut: 'Ctrl+Shift+A', divider: true, disabled: busy, run: () => void command('archive_session', { sessionId: session.id }) },
      { label: '永久删除', icon: Trash2, disabled: busy, danger: true, run: () => openDialog({ kind: 'delete', session }) },
    ];
  }
  function expand(event: { currentTarget: EventTarget & HTMLElement }, item: MenuItem, focus: boolean) {
    if (!item.items || item.disabled) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const main = popup.current!.getBoundingClientRect();
    const right = main.right + 4;
    setSub({ parent: item.label, items: item.items, x: right + 248 < window.innerWidth ? right : main.left - 252, y: bounds.top, focus });
  }
  function renderItems(items: MenuItem[], nested = false) {
    return items.map(item => <div className={item.divider ? 'sidebar-menu-divider' : ''} key={item.id ?? item.label}>
      <button type="button" role={item.checked === undefined ? 'menuitem' : 'menuitemradio'} aria-checked={item.checked} data-submenu={item.items ? item.label : undefined} aria-haspopup={item.items ? 'menu' : undefined} aria-expanded={item.items ? sub?.parent === item.label : undefined} className={item.danger ? 'danger-action' : ''} disabled={working || item.disabled} onPointerEnter={event => { if (!nested && item.items) expand(event, item, false); else if (!nested) setSub(null); }} onClick={event => item.items ? expand(event, item, true) : item.run?.()}>
        {item.icon ? <item.icon size={16}/> : <span className="sidebar-menu-icon"/>}<span title={item.description}>{item.label}{item.description ? <small className="sidebar-submenu-path">{item.description}</small> : null}</span>{item.shortcut ? <kbd>{item.shortcut}</kbd> : null}{item.items ? <ChevronRight size={14}/> : item.checked ? <Check size={14}/> : null}
      </button>
    </div>);
  }
  function navigate(event: React.KeyboardEvent<HTMLDivElement>) {
    const active = document.activeElement as HTMLButtonElement;
    const list = active.closest('[role="menu"]') ?? popup.current;
    const items = [...list!.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')].filter(button => button.closest('[role="menu"]') === list);
    const at = items.indexOf(active);
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) { event.preventDefault(); const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : event.key === 'ArrowDown' ? (at + 1) % items.length : (at + items.length - 1) % items.length; items[next]?.focus(); }
    if (event.key === 'ArrowRight' && active.dataset.submenu && menu) { event.preventDefault(); const item = menuItems(menu.target).find(item => item.label === active.dataset.submenu); if (item) expand({ currentTarget: active }, item, true); }
    if (event.key === 'ArrowLeft' && sub) { event.preventDefault(); const parent = sub.parent; setSub(null); popup.current?.querySelector<HTMLButtonElement>(`[data-submenu="${parent}"]`)?.focus(); }
    if (event.key === 'Tab') closeMenu(true);
  }
  async function saveDialog() {
    if (!dialog) return;
    const value = dialog;
    await run(async () => {
      if (value.kind === 'rename') await call('rename_session', { sessionId: value.session.id, title: title.trim() });
      else if (value.kind === 'edit') await call('edit_sidebar_project', { projectId: value.project.id, name: title.trim(), path: path.trim() });
      else if (value.kind === 'section') {
        const section = await call<SidebarSection>('save_sidebar_section', { id: value.section?.id ?? null, name: title.trim() });
        if (value.target?.kind === 'project' || value.target?.kind === 'session') await call('update_sidebar_item', { kind: value.target.kind, id: value.target.item.id, action: 'section', value: section.id });
      } else if (value.kind === 'delete') await call('delete_sidebar_session', { sessionId: value.session.id });
      else if (value.kind === 'remove' || value.kind === 'archiveProject') await call('sidebar_project_action', { projectId: value.project.id, action: value.kind === 'remove' ? 'remove' : 'archive' });
      await updated();
    });
  }
  async function exportChat(session: Session, format: 'html' | 'md') {
    await run(async () => {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const path = await save({ title: format === 'html' ? '分享聊天' : '导出聊天', defaultPath: exportFilename(session.title, format), filters: [{ name: format === 'html' ? 'HTML' : 'Markdown', extensions: [format] }] });
      if (!path) return;
      const saved = await call<string>('save_chat_export', { sessionId: session.id, path, format });
      setFeedback(`已保存：${saved}`); setMenu(null); setSub(null);
    }, false);
  }
  function chatRow(session: Session, standalone = false) {
    const meta = state.sessions[session.id];
    return <div className={`sidebar-chat-row ${session.id === sessionId ? 'selected' : ''} ${meta?.unread && !isActive(session.status) ? 'unread' : ''}`} key={session.id} onContextMenu={e => show(e, { kind: 'session', item: session })}>
      <button className={`session-row ${session.id === sessionId ? 'selected' : ''}`} aria-current={session.id === sessionId ? 'page' : undefined} onClick={() => void selectSession(session)} title={session.title}>
        <span className="session-title">{session.title}{standalone && session.projectId ? <small>{projects.find(p => p.id === session.projectId)?.name}</small> : null}</span><SessionIndicator status={session.status} unread={meta?.unread}/>{session.status === 'waiting' ? <span className="waiting-badge" aria-label="需要确认">!</span> : null}
      </button>
      <button className="sidebar-row-menu" aria-label={`${session.title} 的菜单`} onClick={e => show(e, { kind: 'session', item: session }, true)}><MoreHorizontal size={15}/></button>
    </div>;
  }
  function toggleProject(id: string) {
    setCollapsed(old => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; });
  }
  function activateProject(project: Project) {
    if (project.id === projectId) return;
    const recent = projectSessions(project.id, sessions, state)[0];
    if (recent) void selectSession(recent); else selectProject(project);
  }
  function projectRow(project: Project) {
    const expanded = !collapsed.has(project.id);
    return <div className="project-group" key={project.id}>
      <div className="project-heading" onContextMenu={e => show(e, { kind: 'project', item: project })}>
        <button className="icon-button project-folder" aria-label={`${expanded ? '收起' : '展开'} ${project.name} 的会话`} title={expanded ? '收起项目会话' : '展开项目会话'} aria-expanded={expanded} onClick={e => { if (e.detail < 2) toggleProject(project.id); }}><span className="project-folder-glyph" aria-hidden="true"><FolderClosed className="project-folder-closed" size={18}/><FolderOpen className="project-folder-open" size={18}/></span></button>
        <button className={`project-row ${project.id === projectId ? 'active' : ''}`} aria-expanded={expanded} onClick={e => { if (e.detail < 2) { toggleProject(project.id); activateProject(project); } }} title={`${project.path} · 单击展开或收起会话`}><span>{project.name}</span></button>
        <button className="sidebar-row-menu" aria-label={`${project.name} 的项目菜单`} onClick={e => show(e, { kind: 'project', item: project }, true)}><MoreHorizontal size={15}/></button>
        <button className="sidebar-row-menu project-new-chat" aria-label={`在 ${project.name} 中新建聊天`} title="新建聊天" onClick={() => void createSession(project)}><SquarePen size={16}/></button>
      </div>
      {expanded ? <div className="session-list">{projectSessions(project.id, sessions, state).map(s => chatRow(s))}</div> : null}
    </div>;
  }
  function renderGroup(group: SidebarGroup, direct = false) {
    return <section className="sidebar-group" key={group.id} aria-label={group.name}>
      {group.id !== 'default' && !(direct && group.id === 'recent') ? <div className="sidebar-group-heading" onContextMenu={e => { const section = state.sections.find(s => s.id === group.id); if (section) show(e, { kind: 'section', item: section }); }}>{group.id === 'pinned' ? <Pin size={12}/> : <List size={12}/>}<span>{group.name}</span>{state.sections.some(s => s.id === group.id) ? <button className="sidebar-row-menu" aria-label={`${group.name} 的分区菜单`} onClick={e => show(e, { kind: 'section', item: state.sections.find(s => s.id === group.id)! }, true)}><MoreHorizontal size={14}/></button> : null}</div> : null}
      {group.projects.map(projectRow)}{group.sessions.map(s => chatRow(s, true))}{!group.projects.length && !group.sessions.length ? <span className="sidebar-group-empty">暂无内容</span> : null}
    </section>;
  }
  const areas = sidebarAreas(projects, sessions, state);
  const dialogTitle = dialog?.kind === 'rename' ? '重命名聊天' : dialog?.kind === 'edit' ? '编辑项目' : dialog?.kind === 'section' ? dialog.section ? '重命名分区' : '新建分区' : dialog?.kind === 'delete' ? '永久删除聊天' : dialog?.kind === 'remove' ? '移除项目' : dialog?.kind === 'archiveProject' ? '归档项目聊天' : dialog?.kind === 'share' ? '分享聊天' : '已归档聊天';
  const shownArchives = archives.filter(row => `${row.session.title} ${row.projectName}`.toLowerCase().includes(archiveSearch.toLowerCase()));
  return <>
    <div className="sidebar-navigation">
      <section className="sidebar-projects sidebar-region" aria-label="项目列表">
        <div className="sidebar-section-label"><span>项目</span><div><button className="icon-button" aria-label="侧栏管理" onClick={e => show(e, { kind: 'root' }, true)}><MoreHorizontal size={15}/></button><button className="icon-button" title="添加项目" onClick={addProject}><Plus size={15}/></button></div></div>
        <div className="project-list sidebar-items">{!ready ? <div className="sidebar-loading" role="status">正在加载项目…</div> : areas.projects.map(group => renderGroup(group))}{ready && !projects.length ? <button className="empty-project" onClick={addProject}><FolderOpen size={22}/><span>添加项目</span></button> : null}</div>
      </section>
      <section className="sidebar-direct sidebar-region" aria-label="直接对话">
        <div className="sidebar-section-label"><span>直接对话</span><div><button className="icon-button" aria-label="新建直接对话" title="新建直接对话" disabled={!ready || blocked} onClick={() => void createSession()}><Plus size={15}/></button></div></div>
        <div className="project-list sidebar-items">{!ready ? <div className="sidebar-loading" role="status">正在加载对话…</div> : areas.direct.length ? areas.direct.map(group => renderGroup(group, true)) : <span className="sidebar-direct-empty">暂无对话</span>}</div>
      </section>
    </div>
    {error && !menu && !dialog ? <div className="sidebar-action-error" role="alert"><span>{error}</span><button className="icon-button" title="关闭提示" onClick={() => setError('')}><X size={13}/></button></div> : null}
    {menu ? createPortal(<div ref={popup} className="sidebar-context-menu" role="menu" aria-label={menu.target.kind === 'project' ? '项目操作' : menu.target.kind === 'session' ? '聊天操作' : '分区操作'} style={{ left: menu.x, top: menu.y }} onKeyDown={navigate}>
      {renderItems(menuItems(menu.target))}{feedback ? <div className="sidebar-menu-feedback" role="status">{feedback}</div> : null}{error ? <div className="resource-menu-error" role="alert">{error}</div> : null}
      {sub ? <div className="sidebar-context-menu sidebar-submenu" role="menu" aria-label={sub.parent} style={{ left: sub.x, top: sub.y }}>{renderItems(sub.items, true)}</div> : null}
    </div>, document.body) : null}
    {dialog ? createPortal(<div className="modal-backdrop sidebar-dialog-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) closeDialog(); }}><section ref={panel} className={`modal sidebar-dialog ${dialog.kind === 'archives' ? 'archive-dialog' : ''}`} role="dialog" aria-modal="true" aria-label={dialogTitle}>
      <div className="modal-header"><h2>{dialogTitle}</h2><button className="icon-button" title="关闭" disabled={working} onClick={closeDialog}><X size={18}/></button></div>
      {dialog.kind === 'rename' || dialog.kind === 'edit' || dialog.kind === 'section' ? <form onSubmit={e => { e.preventDefault(); void saveDialog(); }}><label htmlFor="sidebar-name">名称</label><input id="sidebar-name" value={title} disabled={working} onChange={e => setTitle(e.target.value)} maxLength={100}/>{dialog.kind === 'edit' ? <><label htmlFor="sidebar-path">文件夹</label><div className="sidebar-path-input"><input id="sidebar-path" value={path} disabled={working} onChange={e => setPath(e.target.value)}/><button type="button" className="quiet-button" disabled={!desktop || working} onClick={() => void run(async () => { const { open } = await import('@tauri-apps/plugin-dialog'); const selected = await open({ directory: true, multiple: false, defaultPath: path, title: '选择项目文件夹' }); if (selected) setPath(selected); }, false)}>选择</button></div></> : null}<div className="modal-actions"><button type="button" className="quiet-button" disabled={working} onClick={closeDialog}>取消</button><button className="primary-button" disabled={working || !title.trim() || dialog.kind === 'edit' && !path.trim()}>{working ? '保存中…' : '保存'}</button></div></form> : null}
      {dialog.kind === 'delete' || dialog.kind === 'remove' || dialog.kind === 'archiveProject' ? <><p className="sidebar-confirm-text">{dialog.kind === 'delete' ? `删除“${dialog.session.title}”及其聊天记录？此操作无法撤销。` : dialog.kind === 'remove' ? `从侧栏移除“${dialog.project.name}”？项目文件和聊天记录会保留，重新添加目录即可找回。` : `归档“${dialog.project.name}”中的所有聊天？可以在“已归档”中恢复。`}</p><div className="modal-actions"><button className="quiet-button" disabled={working} onClick={closeDialog}>取消</button><button className={`primary-button ${dialog.kind === 'delete' ? 'sidebar-delete-confirm' : ''}`} disabled={working} onClick={() => void saveDialog()}>{working ? '处理中…' : dialog.kind === 'delete' ? '永久删除' : dialog.kind === 'remove' ? '移除项目' : '归档聊天'}</button></div></> : null}
      {dialog.kind === 'share' ? <><p className="sidebar-share-title">{dialog.session.title}</p><p className="muted">导出聊天文件，可直接发送给别人。</p><div className="sidebar-share-actions"><button className="primary-button" disabled={working || !desktop} onClick={() => void exportChat(dialog.session, 'html')}><Share2 size={15}/>保存 HTML</button><button className="quiet-button" disabled={working || !desktop} onClick={() => void exportChat(dialog.session, 'md')}><Download size={15}/>保存 Markdown</button></div></> : null}
      {dialog.kind === 'archives' ? <><div className="search-field"><Archive size={16}/><input aria-label="搜索归档聊天" value={archiveSearch} onChange={e => setArchiveSearch(e.target.value)} placeholder="搜索聊天或项目"/></div><div className="archive-list">{archiveLoading ? <div className="archive-empty"><LoaderCircle size={16} className="spin"/>加载中…</div> : shownArchives.length ? shownArchives.map(row => <div className="archive-row" key={row.session.id}><div><strong>{row.session.title}</strong><small>{row.projectName}</small></div><button className="quiet-button" disabled={working} onClick={() => void run(async () => { await call('restore_sidebar_session', { sessionId: row.session.id }); await updated(); await loadArchives(); }, false)}>恢复</button><button className="icon-button" title={`永久删除 ${row.session.title}`} disabled={working} onClick={() => openDialog({ kind: 'delete', session: row.session })}><Trash2 size={15}/></button></div>) : <div className="archive-empty">{archiveSearch ? '没有匹配的聊天' : '暂无归档聊天'}</div>}</div></> : null}
      {feedback ? <div className="sidebar-dialog-feedback" role="status">{feedback}</div> : null}{error ? <div className="error-banner" role="alert">{error}</div> : null}
    </section></div>, document.body) : null}
  </>;
}
