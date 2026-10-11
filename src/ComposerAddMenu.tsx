import { useEffect, useRef, useState } from 'react';
import { ArrowLeft, Check, ChevronDown, File, Folder, Image, ListChecks, LoaderCircle, Plus, Puzzle, RefreshCw, Search, Sparkles, Target, X } from 'lucide-react';
import { call, desktop } from './api';
import { imagePath, type Attachment } from './attachments';
import { composerCapabilities, pluginContainsPath } from './composerCapabilities';
import type { ExtensionCatalog } from './extensions';
import { loadLocalSkills, skillSource, type Skill, type SkillCatalog } from './skills';
import type { PermissionMode } from './types';
import { useFloatingLayer } from './useFloatingLayer';
import './composer-add-menu.css';

interface Props {
  projectId: string; agent: string; permissionMode: PermissionMode;
  attachments: Attachment[]; busy: boolean; modeBusy: boolean;
  addAttachments: (items: Attachment[]) => Promise<void>; project?: () => void;
  onSkill: (skill: Skill) => boolean; onPlan: (enabled: boolean) => void;
  onGoal: (objective: string) => void;
}
const emptySkills: SkillCatalog = { skills: [], warnings: [] };
const emptyExtensions: ExtensionCatalog = { plugins: [], mcp: [], warnings: [] };

export function AddMenu({ projectId, agent, permissionMode, attachments, busy, modeBusy, addAttachments, project, onSkill, onPlan, onGoal }: Props) {
  const [open, setOpen] = useState(false);
  const [panel, setPanel] = useState<'menu' | 'goal'>('menu');
  const [skills, setSkills] = useState(emptySkills);
  const [extensions, setExtensions] = useState(emptyExtensions);
  const [catalogError, setCatalogError] = useState('');
  const [loading, setLoading] = useState(false);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [skillsExpanded, setSkillsExpanded] = useState(true);
  const [expandedPlugin, setExpandedPlugin] = useState('');
  const [objective, setObjective] = useState('');
  const [revision, setRevision] = useState(0);
  const goalInput = useRef<HTMLTextAreaElement>(null);
  const layer = useFloatingLayer(open, () => { setOpen(false); setPanel('menu'); }, { focusFirst: true });
  const planSupported = agent === 'codex' || agent === 'claude';
  const planning = planSupported && permissionMode === 'read';
  const catalog = composerCapabilities(skills.skills, extensions.plugins, agent, search);

  useEffect(() => { if (busy) { setOpen(false); setPanel('menu'); } }, [busy]);
  useEffect(() => { if (modeBusy && panel === 'goal') setPanel('menu'); }, [modeBusy, panel]);
  useEffect(() => {
    if (!open || !desktop) return;
    let cancelled = false;
    setLoading(true); setCatalogError('');
    void Promise.allSettled([
      loadLocalSkills(projectId, revision > 0),
      call<ExtensionCatalog>('list_extensions', { projectId: projectId || null, browse: false }),
    ]).then(([local, plugins]) => {
      if (cancelled) return;
      const errors: string[] = [];
      if (local.status === 'fulfilled') setSkills(local.value); else { setSkills(emptySkills); errors.push(`技能读取失败：${String(local.reason)}`); }
      if (plugins.status === 'fulfilled') setExtensions(plugins.value); else { setExtensions(emptyExtensions); errors.push(`插件读取失败：${String(plugins.reason)}`); }
      setCatalogError(errors.join('\n')); setLoading(false);
    });
    return () => { cancelled = true; };
  }, [open, projectId, agent, revision]);
  useEffect(() => {
    if (open && panel === 'goal') goalInput.current?.focus();
  }, [open, panel]);
  useEffect(() => {
    if (!open) return;
    const changed = (event: Event) => {
      const detail = (event as CustomEvent<{ projectId: string; catalog: SkillCatalog }>).detail;
      if (detail.projectId === projectId) setSkills(detail.catalog);
    };
    window.addEventListener('supercode:skills-updated', changed);
    return () => window.removeEventListener('supercode:skills-updated', changed);
  }, [open, projectId]);

  function toggle() {
    setPanel('menu'); setSearch(''); setObjective(''); setError(''); setExpandedPlugin(''); setSkillsExpanded(true);
    setOpen(value => !value);
  }
  async function pick(kind: 'file' | 'image' | 'directory') {
    layer.dismiss(true); setError('');
    if (!desktop) { setError('桌面版可添加本机文件'); return; }
    setPicking(true);
    try {
      const { open: dialog } = await import('@tauri-apps/plugin-dialog');
      const selected = await dialog({ multiple: kind !== 'directory', directory: kind === 'directory', title: kind === 'image' ? '添加图片' : kind === 'directory' ? '添加目录' : '添加文件', filters: kind === 'image' ? [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }] : undefined });
      if (!selected) return;
      await addAttachments((Array.isArray(selected) ? selected : [selected]).map(path => ({ kind: kind === 'file' && imagePath(path) ? 'image' as const : kind, path, name: path.split(/[\\/]/).filter(Boolean).pop() ?? path })));
    } catch (e) { setError(String(e)); } finally { setPicking(false); }
  }
  function choose(skill: Skill) {
    if (busy || skill.error) return;
    const owners = extensions.plugins.filter(plugin => pluginContainsPath(plugin, skill.path));
    if (owners.length && !owners.some(plugin => plugin.enabled && (plugin.agent === agent || plugin.agent === 'both'))) return;
    if (onSkill(skill)) layer.dismiss();
  }
  function startGoal() {
    const text = objective.trim();
    if (!text || modeBusy || busy || agent !== 'codex' || !desktop) return;
    onGoal(text); layer.dismiss();
  }
  function skillRow(skill: Skill, disabled = false) {
    const selected = attachments.some(item => item.kind === 'skill' && item.path === skill.path);
    return <button type="button" className="add-skill-row" role="menuitemcheckbox" aria-checked={selected} key={skill.path}
      disabled={disabled || !!skill.error} title={skill.error || skill.description || skill.name} onClick={() => choose(skill)}>
      <Sparkles size={14}/><span className="add-row-copy"><strong>{skill.name}</strong><small>{skill.error || skill.description || skillSource(skill)}</small></span>
      {selected ? <Check className="add-selected" size={15}/> : <Plus className="add-row-end" size={13}/>}
    </button>;
  }
  return <div className="composer-menu-anchor composer-add-anchor" ref={layer.root} onKeyDown={layer.navigate}>
    <button type="button" className="icon-button" title="添加文件、计划、目标与技能" aria-label="添加附件" aria-haspopup="dialog" aria-expanded={open}
      aria-controls={open ? `${layer.id}-add` : undefined} disabled={busy || picking} onClick={toggle}
      onKeyDown={event => { if (event.key === 'ArrowDown' && !open) { event.preventDefault(); toggle(); } }}><Plus size={18}/></button>
    {open ? <div id={`${layer.id}-add`} className="composer-popover add-popover" role="dialog" aria-label={panel === 'goal' ? '设置目标' : '添加能力'}>
      {panel === 'goal' ? <section className="add-goal-panel">
        <button type="button" className="add-goal-back" role="menuitem" onClick={() => { setPanel('menu'); requestAnimationFrame(() => layer.root.current?.querySelector<HTMLButtonElement>('[data-goal-entry]')?.focus()); }}><ArrowLeft size={15}/>返回</button>
        <div className="add-goal-heading"><Target size={19}/><strong>设置目标</strong></div>
        <p>明确希望完成的结果，Codex 会持续推进这个目标。</p>
        <label htmlFor={`${layer.id}-objective`}>目标内容</label>
        <textarea id={`${layer.id}-objective`} ref={goalInput} rows={3} maxLength={4000} placeholder="例如：完成登录功能，并通过相关测试" value={objective}
          onChange={event => setObjective(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); if (!event.repeat) startGoal(); } }}/>
        {planning ? <small>当前为计划模式，目标会沿用只读权限。</small> : null}
        <div className="add-goal-actions"><button type="button" onClick={startGoal} disabled={!objective.trim() || modeBusy || busy}>设置并开始<ArrowLeft size={13} className="add-goal-arrow"/></button></div>
      </section> : <>
        <div className="add-section-label">添加</div>
        <div role="menu" aria-label="添加与模式" className="add-actions">
          <button type="button" role="menuitem" onClick={() => void pick('file')}><File size={16}/><span>文件</span><small>添加本机文件</small></button>
          <button type="button" role="menuitem" onClick={() => void pick('image')}><Image size={16}/><span>图片</span><small>添加参考图片</small></button>
          <button type="button" role="menuitem" onClick={() => void pick('directory')}><Folder size={16}/><span>文件夹</span><small>添加目录上下文</small></button>
          <button type="button" role="menuitem" data-goal-entry disabled={agent !== 'codex' || modeBusy || !desktop} onClick={() => setPanel('goal')}>
            <Target size={16}/><span>目标</span><small>{agent !== 'codex' ? 'Codex 支持持续追求目标' : modeBusy ? '任务结束后可设置' : '设置要持续追求的目标'}</small><ChevronDown className="add-row-end" size={13}/>
          </button>
          <button type="button" role="menuitemcheckbox" aria-checked={planning} disabled={!planSupported || modeBusy || !desktop} onClick={() => { onPlan(!planning); layer.dismiss(true); }}>
            <ListChecks size={16}/><span>计划模式</span><small>{!planSupported ? '当前 Agent 可在权限中选择只读' : modeBusy ? '任务结束后可切换' : planning ? '退出规划，恢复请求批准' : '先制定方案，再开始执行'}</small>{planning ? <Check className="add-selected" size={15}/> : null}
          </button>
        </div>
        <div className="add-catalog-search"><Search size={14}/><input value={search} onChange={event => setSearch(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') event.preventDefault(); }} aria-label="搜索插件与技能" placeholder="搜索插件与技能"/>
          <button type="button" title="刷新插件与技能" aria-label="刷新插件与技能" disabled={!desktop || loading} onClick={() => setRevision(value => value + 1)}><RefreshCw size={13} className={loading ? 'spin' : ''}/></button>
        </div>
        <div className="add-capabilities" aria-busy={loading}>
          {loading ? <div className="add-menu-status" role="status"><LoaderCircle size={14} className="spin"/>读取插件与技能…</div> : <>
            <div className="add-section-label"><span>插件</span><small>{catalog.plugins.length}</small></div>
            <div role="menu" aria-label="插件">
              {catalog.plugins.map(({ plugin, skills: pluginSkills }) => {
                const expanded = !!search.trim() || expandedPlugin === plugin.id;
                return <div key={plugin.id} className="add-plugin-group">
                  <button type="button" className="add-plugin-row" role="menuitem" aria-expanded={expanded} aria-controls={`${layer.id}-${plugin.id}`} onClick={() => setExpandedPlugin(expandedPlugin === plugin.id ? '' : plugin.id)}>
                    {plugin.icon ? <img className="add-plugin-icon" src={plugin.icon} alt=""/> : <Puzzle className="add-plugin-icon" size={16}/>}
                    <span className="add-row-copy"><strong>{plugin.name}</strong><small>{plugin.description || plugin.components.join(' · ') || '本机插件'}</small></span>
                    {!plugin.enabled ? <small className="add-plugin-disabled">已停用</small> : null}<ChevronDown className={`add-row-end ${expanded ? 'expanded' : ''}`} size={13}/>
                  </button>
                  {expanded ? <div id={`${layer.id}-${plugin.id}`} className="add-plugin-skills" role="group" aria-label={`${plugin.name} 的技能`}>
                    {!plugin.enabled ? <p className="add-menu-status">插件已停用，可在插件设置中启用。</p> : null}
                    {pluginSkills.map(skill => skillRow(skill, !plugin.enabled))}
                    {!pluginSkills.length ? <p className="add-menu-status">{search.trim() ? '没有匹配的技能' : plugin.enabled ? '此插件提供工具能力，发送任务时按需使用。' : '启用后可读取插件技能。'}</p> : null}
                  </div> : null}
                </div>;
              })}
              {!catalog.plugins.length ? <p className="add-menu-status">{!desktop ? '桌面版会读取本机插件' : search.trim() ? '没有匹配的插件' : '当前 Agent 暂无已安装插件'}</p> : null}
            </div>
            <button type="button" className="add-section-toggle" aria-expanded={skillsExpanded || !!search.trim()} aria-controls={`${layer.id}-skills`} onClick={() => setSkillsExpanded(value => !value)}><span>技能</span><small>{catalog.skills.length}</small><ChevronDown size={13} className={skillsExpanded || search.trim() ? 'expanded' : ''}/></button>
            {skillsExpanded || search.trim() ? <div id={`${layer.id}-skills`} role="menu" aria-label="技能">
              {catalog.skills.map(skill => skillRow(skill))}
              {!catalog.skills.length ? <p className="add-menu-status">{!desktop ? '桌面版会读取本机技能' : search.trim() ? '没有匹配的技能' : '暂无独立技能，插件技能可在上方展开'}</p> : null}
            </div> : null}
          </>}
          {catalogError ? <p className="add-menu-error" role="alert">{catalogError}</p> : null}
          {!loading && (skills.warnings?.length || extensions.warnings.length) ? <p className="add-menu-status" title={[...(skills.warnings ?? []), ...extensions.warnings].join('\n')}>部分目录未能读取，可刷新后重试。</p> : null}
        </div>
        <div className="add-menu-footer">{project ? <button type="button" role="menuitem" onClick={() => { layer.dismiss(); project(); }}><Plus size={14}/>添加项目</button> : null}<small>技能会添加到当前消息</small></div>
      </>}
    </div> : null}
    {error ? <div className="attachment-error" role="alert"><span>{error}</span><button type="button" className="icon-button" aria-label="关闭附件提示" onClick={() => setError('')}><X size={13}/></button></div> : null}
  </div>;
}
