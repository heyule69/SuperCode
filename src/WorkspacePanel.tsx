import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ChevronRight, Folder, FolderOpen, GitBranch, LayoutGrid, ListFilter, LoaderCircle, Maximize2, Minimize2, PanelRightClose, RefreshCw, Search, SquarePlus, X } from 'lucide-react';
import { call, desktop } from './api';
import type { Changes, WorkspaceDirectory, WorkspaceEntry } from './types';
import { CopyButton } from './CopyButton';
import { Collapsible } from './Collapsible';
import { useFloatingLayer } from './useFloatingLayer';
import { FileIcon } from './FileIcon';
import './workspace-panel.css';

const DiffView = lazy(() => import('./DiffView'));
const SourceView = lazy(() => import('./SourceView'));
const ChangeReview = lazy(() => import('./ChangeReview'));
const views = [
  { id: 'overview', label: '概览', icon: LayoutGrid },
  { id: 'files', label: '工作空间文件', icon: Folder },
  { id: 'changes', label: '变更', icon: SquarePlus },
] as const;
type View = typeof views[number]['id'];
export interface FilePreview { name: string; text: string; diff?: boolean; line?: number }
interface Props {
  active: boolean; projectId: string; sessionId: string; workspacePath?: string; workspaceName?: string;
  changes: Changes; loading: boolean; preview: FilePreview | null; expanded: boolean;
  refresh: () => void; close: () => void; expand: () => void; closePreview: () => void;
  openFile: (path: string) => void; reviewFile: (path: string) => void; showDiff: (name: string, diff: string) => void;
  reveal: () => void;
}
function Empty({ title, children }: { title: string; children?: React.ReactNode }) {
  return <div className="workspace-empty"><FolderOpen size={27}/><strong>{title}</strong>{children ? <p>{children}</p> : null}</div>;
}
function displayWorkspacePath(path: string) {
  return path.replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\(?=[a-z]:\\)/i, '');
}

export default function WorkspacePanel(props: Props) {
  const { active, projectId, sessionId, workspacePath, changes, loading, preview } = props;
  const locationLabel = workspacePath ? displayWorkspacePath(workspacePath) : '';
  const [view, setView] = useState<View>(() => {
    const saved = localStorage.getItem('supercode.workspace-view');
    return views.some(item => item.id === saved) ? saved as View : 'overview';
  });
  const [menu, setMenu] = useState(false);
  const layer = useFloatingLayer(menu, () => setMenu(false), { focusFirst: true, position: false });
  const [revision, setRevision] = useState(0);
  const previousChanges = useRef(changes);
  const [outputsOpen, setOutputsOpen] = useState(true);
  useEffect(() => { if (!active) setMenu(false); }, [active]);
  useEffect(() => {
    if (previousChanges.current !== changes) { previousChanges.current = changes; setRevision(value => value + 1); }
  }, [changes]);
  useEffect(() => { localStorage.setItem('supercode.workspace-view', view); }, [view]);
  const current = views.find(item => item.id === view)!;
  const changed = changes.files.filter(file => !file.status.includes('D') && !file.path.endsWith('/'));
  const refresh = () => { setRevision(value => value + 1); props.refresh(); };
  return <>
    <div className="workspace-panel-toolbar">
      <span className="workspace-panel-caption">工作空间</span>
      <div>
        <button className="icon-button" title="在资源管理器中打开" aria-label="定位工作空间" disabled={!desktop || !workspacePath} onClick={props.reveal}><FolderOpen size={16}/></button>
        <button className="icon-button" title="刷新工作空间" aria-label="刷新工作空间" disabled={!workspacePath || loading} onClick={refresh}><RefreshCw size={16} className={loading ? 'spin' : ''}/></button>
        <button className="icon-button" title={props.expanded ? '还原工作空间宽度' : '展开工作空间'} aria-label={props.expanded ? '还原工作空间宽度' : '展开工作空间'} aria-pressed={props.expanded} onClick={props.expand}>{props.expanded ? <Minimize2 size={16}/> : <Maximize2 size={16}/>}</button>
        <button className="icon-button" title="收起工作空间" aria-label="收起工作空间" onClick={props.close}><PanelRightClose size={17}/></button>
      </div>
    </div>
    <div className="workspace-view-bar">
      <div className="workspace-view-anchor" ref={layer.root} onKeyDown={layer.navigate}>
        <button className="workspace-view-trigger" aria-label="切换工作空间视图" aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(value => !value)} onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); setMenu(true); } }}>
          <current.icon size={16}/><span>{current.label}</span><ChevronDown size={13} className={menu ? 'turned' : ''}/>
        </button>
        {menu ? <div className="composer-popover workspace-view-menu" role="menu" aria-label="工作空间视图">
          {views.map(item => <button key={item.id} role="menuitemradio" aria-checked={view === item.id} onClick={() => { setView(item.id); props.closePreview(); layer.dismiss(true); }}><item.icon size={16}/><span>{item.label}</span>{view === item.id ? <Check size={15} className="workspace-view-check"/> : null}</button>)}
        </div> : null}
      </div>
    </div>
    {preview ? <div className="file-preview workspace-file-preview">
      <div className="file-preview-header"><FileIcon name={preview.name}/><span title={preview.name}>{preview.name.split(/[\\/]/).pop()}</span><CopyButton text={preview.text} label="复制文件内容" iconOnly/><button className="icon-button" title="关闭文件" onClick={props.closePreview}><X size={14}/></button></div>
      <Suspense fallback={<pre>{preview.text}</pre>}>{preview.diff ? <DiffView text={preview.text}/> : <SourceView text={preview.text} line={preview.line} name={preview.name}/>}</Suspense>
    </div> : <div className={`workspace-panel-content ${view === 'changes' && workspacePath ? 'workspace-change-content' : ''}`} key={view}>
      {!workspacePath ? <Empty title="还没有工作文件">发送任务后会创建独立目录，也可以选择项目。</Empty> : view === 'changes' ? <Suspense fallback={<div className="workspace-loading"><LoaderCircle size={16} className="spin"/>正在打开变更…</div>}><ChangeReview key={`${projectId}:${sessionId}:${workspacePath}`} changes={changes} loading={loading} expanded={props.expanded} expand={props.expand} openFile={props.openFile}/></Suspense> : view === 'files' ? <WorkspaceFiles key={`${projectId}:${sessionId}:${workspacePath}`} active={active} projectId={projectId} sessionId={sessionId} revision={revision} changes={changes} openFile={props.openFile}/> : <>
        <div className="workspace-location" title={locationLabel}><FileIcon name={props.workspaceName ?? locationLabel} directory size={22}/><span><strong>{props.workspaceName ?? '任务工作空间'}</strong><small>{locationLabel}</small>{changes.branch ? <span className="workspace-location-branch"><GitBranch size={11}/>{changes.branch}</span> : null}</span></div>
        <div className="workspace-overview-actions">
          <button onClick={() => setView('files')}><Folder size={15}/><span>浏览文件</span><ChevronRight size={12}/></button>
          <button onClick={() => setView('changes')}><SquarePlus size={15}/><span>查看变更</span><small>{changes.files.length}</small></button>
        </div>
        <button className="workspace-section-heading workspace-section-toggle" aria-expanded={outputsOpen} onClick={() => setOutputsOpen(value => !value)}><span>{changed.length ? '变更文件' : '工作文件'}</span>{changed.length ? <small>{changed.length}</small> : null}<ChevronDown size={12} className={outputsOpen ? '' : 'turned'}/></button>
        <Collapsible open={outputsOpen}>
          {changed.length ? <div className="workspace-file-list">{changed.slice(0, 6).map(file => <button className="workspace-file-row" key={file.path} onClick={() => props.openFile(file.path)} title={file.path}><FileIcon name={file.path}/><span><strong>{file.path.split('/').pop()}</strong>{file.path.includes('/') ? <small>{file.path.slice(0, file.path.lastIndexOf('/'))}</small> : null}</span><small className={`workspace-file-badge ${file.status === '??' || file.status.includes('A') ? 'is-new' : ''}`}>{file.status === '??' || file.status.includes('A') ? '新增' : file.status.includes('R') ? '重命名' : '修改'}</small></button>)}{changed.length > 6 ? <button className="workspace-more-changes" onClick={() => setView('changes')}>查看全部 {changes.files.length} 个变更<ChevronRight size={12}/></button> : null}</div> : <WorkspaceFiles key={`${projectId}:${sessionId}:${workspacePath}`} active={active && outputsOpen} projectId={projectId} sessionId={sessionId} revision={revision} changes={changes} openFile={props.openFile} compact/>}
        </Collapsible>
      </>}
    </div>}
  </>;
}

function WorkspaceFiles({ active, projectId, sessionId, revision, changes, openFile, compact = false }: { active: boolean; projectId: string; sessionId: string; revision: number; changes: Changes; openFile: (path: string) => void; compact?: boolean }) {
  const [filter, setFilter] = useState('');
  return <>
    {!compact ? <label className="workspace-file-search"><Search size={15}/><input aria-label="筛选工作空间文件" placeholder="筛选文件" value={filter} onChange={event => setFilter(event.target.value)}/>{filter ? <button aria-label="清除文件筛选" onClick={() => setFilter('')}><X size={13}/></button> : <ListFilter size={14}/>}</label> : null}
    <Directory revision={revision} path="." active={active} projectId={projectId} sessionId={sessionId} depth={0} filter={filter} changes={changes} openFile={openFile} compact={compact}/>
    {!compact && filter ? <p className="workspace-filter-note">筛选当前已展开的文件夹。</p> : null}
  </>;
}
function Directory({ path, active, projectId, sessionId, depth, filter, changes, openFile, compact, revision }: { path: string; active: boolean; projectId: string; sessionId: string; depth: number; filter: string; changes: Changes; openFile: (path: string) => void; compact: boolean; revision: number }) {
  const [data, setData] = useState<WorkspaceDirectory>();
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const loaded = useRef(-1);
  useEffect(() => {
    if (!active || loaded.current === revision + retry) return;
    let cancelled = false;
    setError('');
    void call<WorkspaceDirectory>('list_workspace_directory', { projectId, sessionId: sessionId || null, path })
      .then(result => { if (!cancelled) { loaded.current = revision + retry; setData(result); } })
      .catch(reason => { if (!cancelled) setError(String(reason)); });
    return () => { cancelled = true; };
  }, [active, path, projectId, sessionId, retry, revision]);
  if (error) return <div className="workspace-directory-error" role="alert"><span>{error}</span><button className="quiet-button" onClick={() => setRetry(value => value + 1)}>重试</button></div>;
  if (!data) return active ? <div className="workspace-loading"><LoaderCircle size={14} className="spin"/>加载文件…</div> : null;
  const files = data.entries.filter(entry => entry.isDirectory || !filter || entry.name.toLocaleLowerCase().includes(filter.toLocaleLowerCase()));
  return <div className="workspace-directory">
    {files.map(entry => <Entry key={entry.path} entry={entry} active={active} projectId={projectId} sessionId={sessionId} depth={depth} filter={filter} changes={changes} openFile={openFile} compact={compact} revision={revision}/>)}
    {!files.length ? depth === 0 ? <Empty title={filter ? '没有匹配的文件' : '还没有文件'}/> : <span className="workspace-folder-empty">{filter ? '没有匹配项' : '空文件夹'}</span> : null}
    {data.truncated ? <p className="workspace-filter-note">此文件夹较大，仅显示前 500 项。</p> : null}
  </div>;
}
function Entry(props: { entry: WorkspaceEntry; active: boolean; projectId: string; sessionId: string; depth: number; filter: string; changes: Changes; openFile: (path: string) => void; compact: boolean; revision: number }) {
  const { entry, depth } = props;
  const [open, setOpen] = useState(false);
  const status = props.changes.files.find(file => file.path === entry.path)?.status;
  return <div className="workspace-tree-entry">
    <button className="workspace-tree-row" style={{ paddingLeft: 10 + Math.min(depth, 12) * 14 }} aria-expanded={entry.isDirectory ? open : undefined} title={entry.path} onClick={() => entry.isDirectory ? setOpen(value => !value) : props.openFile(entry.path)}>
      {entry.isDirectory ? <ChevronRight size={11} className={open ? 'turned' : ''}/> : <span className="workspace-tree-spacer"/>}<FileIcon name={entry.path} directory={entry.isDirectory} expanded={open}/>
      <span>{entry.name}</span>{status ? <small className="workspace-tree-status">{status === '??' ? '新' : status}</small> : null}
    </button>
    {entry.isDirectory ? <Collapsible open={open}><Directory {...props} path={entry.path} active={props.active && open} depth={depth + 1}/></Collapsible> : null}
  </div>;
}
