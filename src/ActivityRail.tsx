import { FolderOpen, Home, Search, Settings2, SquarePen } from 'lucide-react';

export function ActivityRail({ settings, blocked, home, create, search, projects, preferences }: {
  settings: boolean; blocked: boolean; home: () => void; create: () => void;
  search: () => void; projects: () => void; preferences: () => void;
}) {
  return <nav className="activity-rail" aria-label="应用导航" inert={blocked}>
    <button className={!settings ? 'selected' : ''} aria-label="聊天" title="聊天" aria-current={!settings ? 'page' : undefined} onClick={home}><Home size={19}/></button>
    <button aria-label="新建聊天" title="新建聊天 (Ctrl+N)" onClick={create}><SquarePen size={19}/></button>
    <button aria-label="搜索聊天" title="搜索聊天 (Ctrl+K)" onClick={search}><Search size={19}/></button>
    <button aria-label="显示项目" title="显示项目" onClick={projects}><FolderOpen size={19}/></button>
    <div className="activity-rail-spacer"/>
    <button className={settings ? 'selected' : ''} aria-label="设置" title="设置 (Ctrl+,)" aria-current={settings ? 'page' : undefined} onClick={preferences}><Settings2 size={19}/></button>
  </nav>;
}
