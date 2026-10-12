import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { FolderTree, Globe, Lightbulb, Maximize2, MessagesSquare, Minimize2, PanelRightClose, Plus, Users, X } from 'lucide-react';
import type { RightTab, RightTabKind } from './rightPaneState';
import { FileIcon } from './FileIcon';
import { useFloatingLayer } from './useFloatingLayer';
import './right-pane.css';

const icons: Partial<Record<RightTabKind, typeof Globe>> = { workspace: FolderTree, browser: Globe, question: MessagesSquare, subagents: Users };
export default function RightPane({ tabs, active, select, close, closeAll, expanded, expand, openWorkspace, openBrowser, openQuestion, browserEnabled, resizeHandle, hidden, blocked, onMenuVisibility, children }: {
  tabs: RightTab[]; active: string | null; select: (id: string) => void; close: (id: string) => void; closeAll: () => void;
  expanded: boolean; expand: () => void; openWorkspace: () => void; openBrowser: () => void; openQuestion: () => void; browserEnabled: boolean;
  resizeHandle?: ReactNode; hidden: boolean; blocked: boolean; onMenuVisibility: (open: boolean) => void; children: ReactNode;
}) {
  const [menu, setMenu] = useState(false);
  const layer = useFloatingLayer(menu, () => setMenu(false), { focusFirst: true, position: false });
  const strip = useRef<HTMLDivElement>(null);
  const prefix = useId();
  useEffect(() => { if (hidden || blocked) setMenu(false); }, [hidden, blocked]);
  useEffect(() => { onMenuVisibility(menu); }, [menu, onMenuVisibility]);
  useEffect(() => { strip.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' }); }, [active]);
  function move(event: React.KeyboardEvent) {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const current = tabs.findIndex(tab => tab.id === active);
    const index = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (current + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
    const next = tabs[index]; if (!next) return;
    select(next.id); strip.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[index]?.focus({ preventScroll: true });
  }
  return <aside className="right-pane" aria-label="右侧内容栏" hidden={hidden} inert={hidden || blocked}>
    {resizeHandle}
    <header className="right-pane-header">
      <div className="right-pane-tabs" ref={strip} role="tablist" aria-label="已打开的内容" onKeyDown={move}>
        {tabs.map((tab, index) => {
          const Icon = icons[tab.kind];
          return <div key={tab.id} className={`right-pane-tab${active === tab.id ? ' selected' : ''}`}>
            <button type="button" role="tab" id={`${prefix}-tab-${index}`} aria-controls={`${prefix}-panel`} className="right-pane-tab-label" aria-selected={active === tab.id} title={tab.preview?.name ?? tab.title} onClick={() => select(tab.id)}>
              {Icon ? <Icon size={15}/> : tab.preview?.plan ? <Lightbulb size={15}/> : <FileIcon name={tab.title} size={15}/>}<span>{tab.title}</span>
            </button>
            <button type="button" className="icon-button right-pane-tab-close" aria-label={`关闭${tab.title}`} title={tab.preview?.plan ? '关闭计划' : tab.kind === 'document' ? '关闭文件' : `关闭${tab.title}`} onClick={() => close(tab.id)}><X size={13}/></button>
          </div>;
        })}
      </div>
      <div className="right-pane-add" ref={layer.root} onKeyDown={layer.navigate}>
        <button type="button" className="icon-button" aria-label="在右侧打开内容" title="打开内容" aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(value => !value)}><Plus size={17}/></button>
        {menu ? <div className="composer-popover right-pane-menu" role="menu" aria-label="打开右侧内容">
          <button type="button" role="menuitem" onClick={() => { openWorkspace(); layer.dismiss(true); }}><FolderTree size={16}/>工作空间</button>
          <button type="button" role="menuitem" disabled={!browserEnabled} onClick={() => { openBrowser(); layer.dismiss(true); }}><Globe size={16}/>浏览器</button>
          <button type="button" role="menuitem" onClick={() => { openQuestion(); layer.dismiss(true); }}><MessagesSquare size={16}/>侧边提问</button>
        </div> : null}
      </div>
      <button type="button" className="icon-button" aria-label={expanded ? '还原右侧栏宽度' : '展开右侧栏'} aria-pressed={expanded} title={expanded ? '还原宽度' : '展开右侧栏'} onClick={expand}>{expanded ? <Minimize2 size={16}/> : <Maximize2 size={16}/>}</button>
      <button type="button" className="icon-button" aria-label="关闭右侧栏" title="关闭所有右侧标签" onClick={closeAll}><PanelRightClose size={17}/></button>
    </header>
    <div className="right-pane-content" id={`${prefix}-panel`} role="tabpanel" aria-labelledby={active ? `${prefix}-tab-${tabs.findIndex(tab=>tab.id===active)}` : undefined}>{children}</div>
  </aside>;
}
