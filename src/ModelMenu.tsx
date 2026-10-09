import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Check, ChevronDown, LoaderCircle, MessageSquare, RefreshCw, Search, SlidersHorizontal, X } from 'lucide-react';
import { desktop } from './api';
import { filterModelGroups, isKimiSource, modelGroups, modelName, sourceLabel, type ModelOption } from './modelPicker';
import type { AgentProfile, Model, ModelSource } from './types';
import { useFloatingLayer } from './useFloatingLayer';
import { ProviderIcon } from './AgentIcon';
import type { ConnectionOrder } from './providerOrder';

const effortLabels: Record<string, string> = { low: '低', medium: '中', high: '高', xhigh: '极高', max: '最高', ultra: '超高', minimal: '最低', none: '关闭', off: '关闭' };

function ModelSetting({ label, value, options, choose }: { label: string; value: string; options: { value: string; label: string }[]; choose: (value: string) => void }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('focusin', outside);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); };
  }, [open]);
  useLayoutEffect(() => {
    if (open) menu.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus();
  }, [open]);
  function move(event: React.KeyboardEvent) {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const buttons = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button') ?? []);
    if (!buttons.length) { setOpen(true); return; }
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowDown' ? (current + 1) % buttons.length : (current <= 0 ? buttons.length : current) - 1;
    buttons[next].focus();
  }
  return <div className="model-setting-row">
    <span>{label}</span>
    <div className="model-setting-control" ref={root} onKeyDown={move}>
      <button ref={trigger} type="button" className="model-setting-trigger" aria-label={`选择${label}`} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(v => !v)}>
        <span>{(options.find(option => option.value === value)?.label ?? effortLabels[value] ?? value) || '默认'}</span><ChevronDown size={13}/>
      </button>
      {open ? <div ref={menu} className="model-setting-menu" role="menu" aria-label={label}>
        {options.map(option => <button type="button" role="menuitemradio" aria-checked={option.value === value} key={option.value} onClick={() => { setOpen(false); trigger.current?.focus(); choose(option.value); }}>
          <span>{option.label}</span><span className="model-option-check">{option.value === value ? <Check size={14}/> : null}</span>
        </button>)}
      </div> : null}
    </div>
  </div>;
}

function connectionLabel(source: ModelSource) {
  const name = source.connectionName?.trim() || source.providerName;
  const suffix = source.planName ? ` · ${source.planName}` : '';
  return source.providerId !== 'custom' && suffix && name.endsWith(suffix) ? name.slice(0, -suffix.length) : name;
}

interface Props {
  value: string; models: Model[]; profiles: AgentProfile[]; activeProfile?: AgentProfile;
  source?: ModelSource; busy: boolean; loading: boolean; agent: string;
  choose: (option: ModelOption) => Promise<void>; reload: () => void; manage: () => void;
  effort: string; setEffort: (value: string) => void;
  hasConversation: boolean; connectionId?: string; order?: ConnectionOrder;
}

export function ModelMenu({ value, models, profiles, activeProfile, source, busy, loading, agent, choose, reload, manage, effort, setEffort, hasConversation, connectionId, order }: Props) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<string | null>(null);
  const [left, setLeft] = useState(0);
  const [listHeight, setListHeight] = useState<number>();
  const [showNote, setShowNote] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false), { position: false });
  const anchor = layer.root;
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const note = useRef<HTMLDivElement>(null);
  const groups = modelGroups({ value, models, profiles, activeProfile, source, agent, connectionId, order });
  const currentId = connectionId ?? activeProfile?.id ?? '';
  const currentGroup = groups.find(g => g.id === currentId || g.options.some(option => option.connectionId === currentId));
  const selected = currentGroup?.options.find(m => m.selected);
  const selectedSource = currentGroup?.source;
  const selectedName = selected?.displayName ?? modelName({ id: value, model: value, displayName: value || '选择模型', isDefault: false }, selectedSource);
  const visible = filterModelGroups(groups, search, filter);
  const available = groups.filter(g => g.options.length > 0);
  const efforts = selected?.supportedReasoningEfforts?.map(e => e.reasoningEffort) ?? (agent === 'claude' ? ['low', 'medium', 'high', 'max'] : []);
  const contextVariants = selected && isKimiSource(selected.source) && agent === 'claude' && selected.rawIds.some(id => /^k3\[1m\]$/i.test(id)) && selected.rawIds.includes('k3') ? selected.rawIds : [];

  function close(focus = false) { setOpen(false); setShowNote(false); if (focus) trigger.current?.focus(); }
  useEffect(() => { if (busy) setOpen(false); }, [busy]);
  useEffect(() => { if (!open) setShowNote(false); }, [open]);
  useEffect(() => {
    if (!showNote) return;
    const outside = (event: Event) => { if (!note.current?.contains(event.target as Node)) setShowNote(false); };
    document.addEventListener('pointerdown', outside); document.addEventListener('focusin', outside);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('focusin', outside); };
  }, [showNote]);
  useLayoutEffect(() => {
    if (!open) return;
    function position() {
      const parent = anchor.current?.getBoundingClientRect();
      const popup = popover.current?.getBoundingClientRect();
      const scroll = list.current?.getBoundingClientRect();
      if (parent && popup && scroll) {
        setLeft(Math.max(12, Math.min(parent.left, window.innerWidth - popup.width - 12)) - parent.left);
        const controls = popup.height - scroll.height;
        setListHeight(Math.max(65, Math.min(264, parent.top - 22 - controls)));
      }
    }
    position(); window.addEventListener('resize', position);
    const observer = new ResizeObserver(position); if (popover.current) observer.observe(popover.current);
    return () => { window.removeEventListener('resize', position); observer.disconnect(); };
  }, [open, models, profiles, search, filter, contextVariants.length, efforts.length, hasConversation]);

  async function select(option: ModelOption) {
    close(true);
    await choose(option);
  }
  function move(event: React.KeyboardEvent) {
    if (event.key === 'Enter' && event.target === searchInput.current && !event.nativeEvent.isComposing) { event.preventDefault(); const options = visible.flatMap(g => g.options); const option = options.find(o => o.selected) ?? options[0]; if (option) void select(option); return; }
    if (event.target !== searchInput.current && !list.current?.contains(event.target as Node)) return;
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    if (event.target === searchInput.current && ['Home', 'End'].includes(event.key)) return;
    const options = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button.model-option:not(:disabled)') ?? []);
    if (!options.length) return;
    event.preventDefault();
    const current = options.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1 : event.key === 'ArrowDown' ? (current + 1) % options.length : (current <= 0 ? options.length : current) - 1;
    options[next].focus();
  }
  return <div className="composer-menu-anchor" ref={anchor}>
    <button ref={trigger} type="button" className="composer-chip model-chip" aria-label="选择模型" aria-haspopup="dialog" aria-expanded={open} disabled={busy}
      title={[selectedName, selectedSource && sourceLabel(selectedSource), value && `模型 ID：${value}`].filter(Boolean).join('\n')}
      onClick={() => { setOpen(v => !v); setSearch(''); setFilter(null); if (!open && !models.length && desktop && !loading) reload(); }}>
      {selectedSource?.mark ? <span className="model-provider-mark" aria-hidden="true"><ProviderIcon provider={selectedSource.providerId} name={selectedSource.connectionName || selectedSource.providerName} mark={selectedSource.mark}/></span> : null}
      <span className="model-chip-name">{selectedName}</span>
      {selectedSource?.providerName ? <span className="model-chip-provider">{selectedSource.providerName}</span> : null}
      <ChevronDown size={11} />
    </button>
    {open ? <div ref={popover} className="composer-popover model-popover" style={{ left }} role="dialog" aria-label="模型选择" onKeyDown={move}>
      <div className="model-menu-heading"><h2>选择模型</h2><span title={selectedName}>{selectedName}</span><button type="button" className="icon-button" aria-label="关闭模型选择" onClick={() => close(true)}><X size={15}/></button></div>
      <div className="model-picker-search search-field">
        <Search size={15}/><input ref={searchInput} autoFocus aria-label="搜索模型菜单" placeholder="搜索模型或供应商" value={search} onChange={e => setSearch(e.target.value)} />
        <button type="button" className="icon-button" aria-label="刷新模型" title="刷新模型" disabled={loading || busy || !desktop} onClick={reload}>{loading ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}</button>
      </div>
      {available.length > 1 ? <div className="model-provider-filters" role="group" aria-label="按供应商筛选">
        <button type="button" aria-pressed={filter === null} onClick={() => setFilter(null)}>全部</button>
        {available.map(g => <button type="button" key={g.id} aria-pressed={filter === g.id} title={sourceLabel(g.source)} onClick={() => setFilter(g.id)}><ProviderIcon provider={g.source.providerId} name={connectionLabel(g.source)} mark={g.source.mark}/><span>{connectionLabel(g.source)}</span></button>)}
      </div> : null}
      <div ref={list} className="model-menu-scroll" style={{ maxHeight: listHeight }} role="menu" aria-label="可用模型">
        {visible.map(group => <div className="model-source-group" key={group.id}>
          <div className="model-group-heading" title={sourceLabel(group.source)}><span><span className="model-group-mark" aria-hidden="true"><ProviderIcon provider={group.source.providerId} name={connectionLabel(group.source)} mark={group.source.mark}/></span><span>{connectionLabel(group.source)}</span></span><small>{group.source.planName}</small></div>
          {group.options.map(option => <button type="button" key={option.key} className={`model-option ${option.selected ? 'selected' : ''}`} role="menuitemradio" aria-checked={option.selected} disabled={busy}
            title={`模型 ID：${option.rawIds.join(' / ')}${option.context === '1M' ? '\n已配置 Claude Code 1M 上下文参数' : ''}`}
            onClick={() => void select(option)}>
            <span className="model-option-copy"><strong>{option.displayName}</strong>{option.context ? <span className="model-context-badge">{option.context}</span> : null}</span>
            <span className="model-option-check">{option.selected ? <Check size={15} /> : null}</span>
          </button>)}
        </div>)}
        {!visible.length ? <div className="model-menu-empty">{loading ? '正在加载模型…' : search || filter !== null ? '没有匹配的模型' : '尚未配置模型'}</div> : null}
      </div>
      <div className="model-menu-footer">
        {selected && contextVariants.length > 1 ? <ModelSetting label="上下文" value={selected.model} options={contextVariants.map(id => ({ value: id, label: /\[1m\]$/i.test(id) ? '1M' : '默认' }))} choose={model => void choose({ ...selected, model, defaultReasoningEffort: effort })}/> : null}
        {efforts.length ? <ModelSetting label="推理强度" value={effort} options={[{ value: '', label: '默认' }, ...efforts.map(value => ({ value, label: effortLabels[value] ?? value }))]} choose={setEffort}/> : null}
        <div className="model-menu-actions">
          <button type="button" onClick={() => { close(); manage(); }}><SlidersHorizontal size={14}/>模型设置</button>
          {hasConversation ? <div ref={note} className="model-context-note"><button type="button" aria-expanded={showNote} onClick={() => setShowNote(v => !v)}><MessageSquare size={14}/>保留当前聊天</button>{showNote ? <div className="model-switch-note" role="note">切换模型会保留当前聊天。上下文较长或需要跨 Agent 交接时，按需压缩。</div> : null}</div> : null}
        </div>
      </div>
    </div> : null}
  </div>;
}
