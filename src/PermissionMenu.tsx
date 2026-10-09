import { useEffect, useState } from 'react';
import { ArrowLeft, Check, ChevronDown, Eye, FileCheck2, Hand, ShieldAlert, ShieldX, SquareTerminal } from 'lucide-react';
import type { PermissionMode } from './types';
import { permissionDescription, permissionDetails, permissionName, permissionOrder, visiblePermission } from './permissions';
import { useFloatingLayer } from './useFloatingLayer';

const icons = { read: Eye, strict: Hand, ask: Hand, auto: SquareTerminal, edit: FileCheck2, deny: ShieldX, full: ShieldAlert };
type Panel = 'options' | 'help' | 'confirm';

export function PermissionMenu({ value, onChange, busy, agent = 'claude', label = '权限模式' }: {
  value: PermissionMode; onChange: (mode: PermissionMode) => void; busy: boolean; agent?: string; label?: string;
}) {
  const [open, setOpen] = useState(false);
  const [panel, setPanel] = useState<Panel>('options');
  const layer = useFloatingLayer(open, () => { setOpen(false); setPanel('options'); }, { focusFirst: true });
  useEffect(() => { if (busy) { setOpen(false); setPanel('options'); } }, [busy]);
  useEffect(() => {
    if (!open) return;
    if (panel !== 'options') layer.root.current?.querySelector<HTMLButtonElement>('.permission-panel button')?.focus();
    else (layer.root.current?.querySelector<HTMLButtonElement>('[role="menuitemradio"][aria-checked="true"]') ?? layer.root.current?.querySelector<HTMLButtonElement>('[role="menuitemradio"]'))?.focus();
  }, [open, panel]);
  function choose(mode: PermissionMode) {
    if (busy) return;
    if (mode === 'full' && value !== 'full') { setPanel('confirm'); return; }
    onChange(mode); layer.dismiss(true);
  }
  const Icon = icons[value];
  const selected = visiblePermission(value, agent);
  return <div className="composer-menu-anchor permission-menu" ref={layer.root} onKeyDown={layer.navigate}>
    <button type="button" className={`composer-chip permission-chip ${value === 'full' ? 'full-access' : ''}`}
      aria-label={label} title={`${permissionName(selected, agent)} · ${permissionDescription(value, agent)}`}
      aria-haspopup="menu" aria-expanded={open} aria-controls={open ? `${layer.id}-permissions` : undefined} disabled={busy}
      onClick={() => { setOpen(v => !v); setPanel('options'); }}
      onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); setOpen(true); setPanel('options'); } }}>
      <Icon size={15}/><span>{permissionName(selected, agent, true)}</span><ChevronDown size={11}/>
    </button>
    {open ? <div id={`${layer.id}-permissions`} className="composer-popover permission-popover"
      role={panel === 'options' ? 'menu' : 'dialog'} aria-label={panel === 'confirm' ? '启用完全访问权限' : panel === 'help' ? '权限说明' : '权限模式'}>
      {panel === 'options' ? <>
        <div className="permission-menu-heading"><span>应如何批准 Agent 操作？</span><button type="button" className="permission-help-link" onClick={() => setPanel('help')}>了解更多</button></div>
        {!permissionOrder(agent).includes(selected) ? <p className="permission-legacy-note">当前保留已保存的{permissionName(value, agent)}配置。选择下方模式后再切换。</p> : null}
        {permissionOrder(agent).map(mode => { const ModeIcon = icons[mode]; return <button type="button" key={mode}
          className={`permission-option ${mode === 'full' ? 'permission-option-full' : ''}`} role="menuitemradio" aria-checked={selected === mode} onClick={() => choose(mode)}>
          <ModeIcon size={20}/><span className="permission-option-copy"><strong>{permissionName(mode, agent)}</strong><small>{permissionDescription(mode, agent)}</small></span>
          <span className="permission-option-check">{selected === mode ? <Check size={17}/> : null}</span>
        </button>; })}
      </> : panel === 'help' ? <div className="permission-panel permission-help">
        <button type="button" className="permission-back" onClick={() => setPanel('options')}><ArrowLeft size={15}/>权限说明</button>
        <p>{permissionDetails(agent)}</p><p>完全访问会取消工作区限制并跳过工具审批。Agent 发出的提问和授权请求仍会显示给你。</p>
        <p>只读用于分析和读取文件，不修改工作区。</p><p>切换影响下一次任务；已保存的权限选择会保留。</p>
      </div> : <div className="permission-panel full-access-confirm">
        <div className="permission-confirm-heading"><ShieldAlert size={21}/><strong>启用完全访问权限</strong></div>
        <p>Agent 可以访问工作区外的文件、互联网和本机程序，工具操作将不再请求批准。</p>
        <div className="permission-confirm-actions"><button type="button" className="quiet-button" onClick={() => setPanel('options')}>返回</button>
          <button type="button" className="primary-button" onClick={() => { if (!busy) { onChange('full'); layer.dismiss(true); } }}>启用完全访问</button></div>
      </div>}
    </div> : null}
  </div>;
}
