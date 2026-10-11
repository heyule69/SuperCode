import { memo, useMemo, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, FileSearch, FolderOpen, GitBranch, ListCollapse, LoaderCircle, Search, WrapText } from 'lucide-react';
import type { Changes } from './types';
import { reviewFiles, reviewHunks, type ReviewFile } from './changeReviewModel';
import { highlightedLines } from './codeHighlight';
import { FileIcon } from './FileIcon';
import './change-review.css';
import './code-syntax.css';

interface Props { changes: Changes; loading: boolean; expanded: boolean; expand: () => void; openFile: (path: string) => void }
export default function ChangeReview({ changes, loading, expanded, expand, openFile }: Props) {
  const files = useMemo(() => reviewFiles(changes), [changes]);
  const [opened, setOpened] = useState<Set<string>>(() => new Set());
  const [selected, setSelected] = useState('');
  const [tree, setTree] = useState(false);
  const [wrap, setWrap] = useState(true);
  const [filter, setFilter] = useState('');
  const [closedFolders, setClosedFolders] = useState<Set<string>>(() => new Set());
  const cards = useRef(new Map<string, HTMLElement>());
  const totals = useMemo(() => files.reduce((sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }), { added: 0, removed: 0 }), [files]);
  const groups = useMemo(() => {
    const result = new Map<string, ReviewFile[]>();
    for (const file of files) {
      if (!file.path.toLocaleLowerCase().includes(filter.toLocaleLowerCase())) continue;
      const lastSlash = file.path.lastIndexOf('/');
      const directory = lastSlash < 0 ? '.' : file.path.slice(0, lastSlash) || '.';
      const group = result.get(directory) ?? []; group.push(file); result.set(directory, group);
    }
    return [...result];
  }, [files, filter]);
  const anyOpen = files.some(file => opened.has(file.path));
  function toggle(path: string, fromTree = false) {
    setSelected(path);
    setOpened(previous => { const next = new Set(previous); if (!fromTree && next.has(path)) next.delete(path); else next.add(path); return next; });
    if (!expanded) expand();
    if (fromTree) cards.current.get(path)?.scrollIntoView({ block: 'nearest', behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });
  }
  function toggleAll() {
    setOpened(anyOpen ? new Set() : new Set(files.filter(file => !file.path.endsWith('/')).map(file => file.path)));
    if (!anyOpen && !expanded) expand();
  }
  function toggleFolder(path: string) { setClosedFolders(previous => { const next = new Set(previous); if (next.has(path)) next.delete(path); else next.add(path); return next; }); }
  return <div className={`change-review ${wrap ? 'is-wrapped' : ''}`} aria-label="变更审阅">
    <div className="change-review-toolbar">
      <div className="change-review-summary">
        <div className="change-review-pill" title="Git 文本差异的增删行数，新文件单独标记"><span>{changes.isGit ? '工作区' : '工作文件'}</span>{totals.added || totals.removed ? <ChangeCount added={totals.added} removed={totals.removed}/> : <small>{files.length}</small>}</div>
        {changes.branch ? <div className="change-review-pill change-review-branch" title={changes.branch}><GitBranch size={13}/><span>{changes.branch}</span></div> : null}
      </div>
      <div className="change-review-tools" aria-label="差异显示选项">
        <button className="icon-button" aria-label={anyOpen ? '折叠全部文件' : '展开全部文件'} title={anyOpen ? '折叠全部文件' : '展开全部文件'} disabled={!files.length} onClick={toggleAll}><ListCollapse size={16}/></button>
        <button className="icon-button" aria-label="代码自动换行" title="代码自动换行" aria-pressed={wrap} onClick={() => setWrap(value => !value)}><WrapText size={16}/></button>
        <button className="icon-button" aria-label="变更文件树" title={tree ? '隐藏变更文件树' : '显示变更文件树'} aria-pressed={tree} onClick={() => setTree(value => !value)}><FolderOpen size={16}/></button>
      </div>
    </div>
    <div className={`change-review-body ${tree ? 'with-tree' : ''}`}>
      <div className="change-review-scroll">
        {loading && !files.length ? <div className="workspace-loading"><LoaderCircle size={16} className="spin"/>正在检查变更…</div> : files.length ? files.map(file => <section key={file.path} className={`change-file ${selected === file.path ? 'is-selected' : ''}`} ref={element => { if (element) cards.current.set(file.path, element); else cards.current.delete(file.path); }}>
          <div className="change-file-header">
            <button className="change-file-toggle" onClick={() => toggle(file.path)} aria-expanded={opened.has(file.path)} aria-label={`${opened.has(file.path) ? '折叠' : '展开'} ${file.path}`} title={file.path} disabled={file.path.endsWith('/')}>
              <FileIcon name={file.path} directory={file.path.endsWith('/')}/><FilePath path={file.path}/><ChevronRight size={13} className={opened.has(file.path) ? 'is-open' : ''}/>
            </button>
            {file.diff && (file.added || file.removed) ? <ChangeCount added={file.added} removed={file.removed}/> : <span className="change-file-status">{file.status === '??' ? '新' : file.status}</span>}
            <button className="icon-button change-source-button" aria-label={`查看 ${file.path} 文件`} title="查看文件内容" disabled={file.status.includes('D') || file.path.endsWith('/')} onClick={() => openFile(file.path)}><FileSearch size={15}/></button>
          </div>
          {opened.has(file.path) ? file.diff ? <ReviewDiff text={file.diff} path={file.path}/> : <div className="change-file-note"><span>{file.status === '??' ? '新文件，尚未纳入 Git' : '没有可显示的文本差异'}</span>{!file.status.includes('D') && !file.path.endsWith('/') ? <button className="quiet-button" onClick={() => openFile(file.path)}>查看文件</button> : null}</div> : null}
        </section>) : <div className="workspace-empty"><FolderOpen size={27}/><strong>{changes.isGit ? '暂无变更' : '未启用 Git'}</strong><p>{changes.isGit ? '修改文件后会显示在这里。' : '文件仍可在“工作空间文件”中浏览。'}</p></div>}
      </div>
      {tree ? <aside className="change-review-tree" aria-label="变更文件树导航">
        <label className="workspace-file-search change-tree-search"><Search size={14}/><input aria-label="筛选变更文件" placeholder="筛选文件…" value={filter} onChange={event => setFilter(event.target.value)}/></label>
        <div className="change-tree-scroll">{groups.map(([directory, group]) => <div className="change-tree-group" key={directory}>
          <button className="change-tree-folder" aria-expanded={!closedFolders.has(directory)} onClick={() => toggleFolder(directory)} title={directory}><ChevronRight size={12} className={closedFolders.has(directory) ? '' : 'is-open'}/><FileIcon name={directory} directory expanded={!closedFolders.has(directory)}/><span>{directory === '.' ? '根目录' : directory}</span></button>
          {!closedFolders.has(directory) ? group.map(file => <button key={file.path} className={`change-tree-file ${selected === file.path ? 'is-selected' : ''}`} title={file.path} aria-label={`定位差异 ${file.path}`} disabled={file.path.endsWith('/')} onClick={() => toggle(file.path, true)}><FileIcon name={file.path} directory={file.path.endsWith('/')}/><span>{file.path.split('/').pop()}</span>{file.added || file.removed ? <ChangeCount added={file.added} removed={file.removed}/> : <small className="change-file-status">{file.status === '??' ? '新' : file.status}</small>}</button>) : null}
        </div>)}{!groups.length ? <div className="change-file-note">{filter ? '没有匹配的变更文件' : '没有变更文件'}</div> : null}</div>
      </aside> : null}
    </div>
    <div className="change-review-footer"><span>{files.length} 个{changes.isGit ? '变更' : '工作'}文件</span><span>{changes.isGit ? '未提交' : '本地文件'}</span></div>
  </div>;
}
function ChangeCount({ added, removed }: { added: number; removed: number }) {
  return <span className="change-count" aria-label={`新增 ${added} 行，删除 ${removed} 行`}>{added ? <span className="change-added">+{added}</span> : null}{removed ? <span className="change-removed">−{removed}</span> : null}</span>;
}
function FilePath({ path }: { path: string }) {
  const split = path.lastIndexOf('/');
  return <span className="change-file-path"><span>{path.slice(0, split + 1)}</span>{path.slice(split + 1)}</span>;
}
export const ReviewDiff = memo(function ReviewDiff({ text, path = '' }: { text: string; path?: string }) {
  const { hunks, truncated, binary, noFinalNewline } = useMemo(() => reviewHunks(text), [text]);
  const highlighted = useMemo(() => hunks.map(hunk => highlightedLines(hunk.lines.map(line => line.text).join('\n'), path)), [hunks, path]);
  const [context, setContext] = useState<Set<number>>(() => new Set());
  return <div className="change-file-diff code-syntax" aria-label="文件代码差异">
    {hunks.map((hunk, index) => <div className="change-hunk" key={index}>
      <div className="change-context-bar">{hunk.context ? <button aria-expanded={context.has(index)} aria-label={`${context.has(index) ? '收起' : '展开'}第 ${hunk.nextStart || hunk.oldStart} 行附近的上下文`} onClick={() => setContext(previous => { const next = new Set(previous); if (next.has(index)) next.delete(index); else next.add(index); return next; })}><ChevronDown size={13} className={context.has(index) ? 'is-open' : ''}/><span>{context.has(index) ? '收起' : '展开'} {hunk.context} 行上下文</span><small>第 {hunk.nextStart || hunk.oldStart} 行附近</small></button> : <span>第 {hunk.nextStart || hunk.oldStart} 行附近</span>}</div>
      {hunk.lines.map((line, row) => line.kind === 'neutral' && !context.has(index) ? null : <div className={`change-code-line ${line.kind}`} key={row}><span className="change-line-number" aria-hidden="true">{line.old ?? ''}</span><span className="change-line-number" aria-hidden="true">{line.next ?? ''}</span><span className="change-line-sign" aria-hidden="true">{line.kind === 'added' ? '+' : line.kind === 'removed' ? '−' : ''}</span><code dangerouslySetInnerHTML={{__html:highlighted[index][row]||' '}}/></div>)}
    </div>)}
    {!hunks.length ? <div className="change-file-note">{binary ? '二进制文件变更，无法显示文本差异' : '仅文件路径、权限或其他信息变更'}</div> : null}
    {noFinalNewline ? <div className="change-diff-note">文件末尾没有换行符</div> : null}
    {truncated ? <div className="change-diff-note">差异较大，仅显示前 5000 行。</div> : null}
  </div>;
});
