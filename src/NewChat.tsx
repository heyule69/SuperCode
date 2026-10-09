import { useState } from 'react';
import { Check, Folder, Plus } from 'lucide-react';
import type { Project } from './types';
import { useFloatingLayer } from './useFloatingLayer';

export function NewChat({ project, projects, selectProject, addProject }: {
  project?: Project; projects: Project[]; selectProject: (project: Project) => void; addProject: () => void;
}) {
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer<HTMLSpanElement>(open, () => setOpen(false), { focusFirst: true });

  return <div className="new-chat-welcome">
    <img className="new-chat-logo" src="/app-icon.png" alt="SuperCode" width={64} height={64} draggable={false}/>
    <h1>你想让我们在{' '}
      <span className="new-chat-project-anchor" ref={layer.root} onKeyDown={layer.navigate}>
        <button type="button" className="new-chat-project" aria-label="选择项目" aria-haspopup="menu"
          aria-expanded={open} aria-controls={open ? `${layer.id}-projects` : undefined} title={project?.path}
          onClick={() => setOpen(value => !value)} onKeyDown={event => {
            if (event.key === 'ArrowDown') { event.preventDefault(); setOpen(true); }
          }}>{project?.name ?? 'SuperCode'}</button>
        {open ? <span id={`${layer.id}-projects`} className="composer-popover new-chat-projects" role="menu" aria-label="项目">
          {projects.map(item => <button type="button" key={item.id} role="menuitemradio" aria-checked={item.id === project?.id}
            title={item.path} onClick={() => { layer.dismiss(true); if (item.id !== project?.id) selectProject(item); }}>
            <Folder size={16}/><span>{item.name}</span>{item.id === project?.id ? <Check size={15}/> : null}
          </button>)}
          <button type="button" className="new-chat-add-project" role="menuitem" onClick={() => { layer.dismiss(true); addProject(); }}>
            <Plus size={16}/><span>添加项目</span>
          </button>
        </span> : null}
      </span>{' '}中构建什么？
    </h1>
  </div>;
}
