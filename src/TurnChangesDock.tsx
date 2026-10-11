import { memo, useMemo, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { currentTurnChanges } from './turnChanges';
import { useFloatingLayer } from './useFloatingLayer';
import { FileIcon } from './FileIcon';
import type { Message } from './types';
import './turn-changes-dock.css';

interface Props { messages: Message[]; activeTurnId?: string | null; showDiff: (name: string, diff: string) => void }
export default memo(function TurnChangesDock({ messages, activeTurnId, showDiff }: Props) {
  const changes = useMemo(() => currentTurnChanges(messages, activeTurnId), [messages, activeTurnId]);
  return changes.files.length ? <ChangeSummary key={changes.id} changes={changes} showDiff={showDiff}/> : null;
});
function ChangeSummary({ changes, showDiff }: { changes: ReturnType<typeof currentTurnChanges>; showDiff: Props['showDiff'] }) {
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false), { focusFirst: true });
  const added = changes.files.reduce((sum, file) => sum + file.added, 0);
  const removed = changes.files.reduce((sum, file) => sum + file.removed, 0);
  function review(path?: string, diff = changes.diff) { layer.dismiss(true); showDiff(path ? `本轮变更 · ${path}` : '本轮变更', diff); }
  return <div className="turn-change-dock" aria-label="当前轮次变更">
    <div className="turn-change-anchor" ref={layer.root} onKeyDown={layer.navigate}>
      <button className="turn-change-chip" aria-label={`本轮 ${changes.files.length} 个文件已更改，新增 ${added} 行，删除 ${removed} 行`} aria-expanded={open} aria-controls={open ? layer.id : undefined} aria-haspopup="menu" onClick={() => setOpen(value => !value)} onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); setOpen(true); } }}>
        <span>{changes.files.length} 个文件已更改</span><span className="turn-change-numbers"><span className="change-added">+{added}</span><span className="change-removed">−{removed}</span></span><ChevronDown size={13} className={open ? 'is-open' : ''}/>
      </button>
      {open ? <div id={layer.id} className="composer-popover turn-change-popover" role="menu" aria-label="本轮变更文件">
        <div className="turn-change-popover-heading">本轮变更</div>
        {changes.files.map(file => <button key={file.path} role="menuitem" title={file.path} aria-label={`查看本轮 ${file.path} 变更`} onClick={() => review(file.path, file.diff)}><FileIcon name={file.path}/><span className="turn-change-filename">{file.path}</span><span className="turn-change-numbers"><span className="change-added">+{file.added}</span><span className="change-removed">−{file.removed}</span></span></button>)}
        <hr/><button role="menuitem" className="turn-change-view-all" onClick={() => review()}>查看全部本轮变更</button>
      </div> : null}
    </div>
  </div>;
}
