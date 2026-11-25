import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronDown, LoaderCircle, Plus, RefreshCw, Search, SlidersHorizontal, X } from 'lucide-react';
import { call, desktop } from './api';
import { magpieGatewayText, useMagpieGateway } from './useMagpieGateway';
import { useMagpieCatalogs } from './useMagpieCatalogs';
import { useOfficialCatalogs } from './useOfficialCatalogs';
import { useCodexSpeeds } from './useCodexSpeeds';
import { filterModelGroups, isKimiSource, modelGroups, sourceLabel, type ModelOption } from './modelPicker';
import type { AgentProfile, Model, ModelSource } from './types';
import { useFloatingLayer } from './useFloatingLayer';
import { ProviderIcon } from './AgentIcon';
import type { ConnectionOrder } from './providerOrder';
import { MODEL_PAGE_SIZE, ModelPagination, pageModels } from './ModelPagination';
import { effortLabels, modelReasoningEfforts, modelReasoningKnown } from './modelReasoning';
import { CodexReasoningSlider } from './CodexReasoningSlider';

function ModelSetting({ label, value, options, choose }: { label: string; value: string; options: { value: string; label: string; disabled?: boolean; title?: string }[]; choose: (value: string) => void }) {
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
    if (open) (menu.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]:not(:disabled)') ?? menu.current?.querySelector<HTMLButtonElement>('button:not(:disabled)'))?.focus({ preventScroll: true });
  }, [open]);
  function move(event: React.KeyboardEvent) {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const buttons = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []);
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
        {options.map(option => <button type="button" role="menuitemradio" aria-checked={option.value === value} disabled={option.disabled} title={option.title} key={option.value} onClick={() => { setOpen(false); trigger.current?.focus({ preventScroll: true }); choose(option.value); }}>
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
  value: string; models: Model[]; catalogLoaded?: boolean; catalogWarning?: string; profiles: AgentProfile[]; activeProfile?: AgentProfile;
  source?: ModelSource; busy: boolean; loading: boolean; agent: string;
  choose: (option: ModelOption) => Promise<void>; reload: (force?: boolean) => void; manage: () => void;
  effort: string; setEffort: (value: string) => void;
  serviceTier?: string | null; setServiceTier?: (value: string) => void;
  hasConversation: boolean; connectionId?: string; order?: ConnectionOrder; codexRevision?: string;
}

export function ModelMenu({ value, models, catalogLoaded, catalogWarning, profiles, activeProfile, source, busy, loading, agent, choose, reload, manage, effort, setEffort, serviceTier, setServiceTier, hasConversation, connectionId, order, codexRevision }: Props) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<string | null>(null);
  const [page, setPage] = useState(0);
  const pendingFocus = useRef<string | null>(null);
  const [left, setLeft] = useState(0);
  const [listHeight, setListHeight] = useState<number>();
  const [accounts, setAccounts] = useState<{ agent: string; ids: string[] }>({ agent: '', ids: [] });
  const loggedInAgents = accounts.agent === agent ? accounts.ids : [];
  const checkingAccounts = desktop && ['claude', 'codex', 'gemini'].includes(agent) && accounts.agent !== agent;
  const [accountWarning, setAccountWarning] = useState('');
  const [accountRevision, setAccountRevision] = useState(0);
  const refreshedAccount = useRef(0);
  const layer = useFloatingLayer(open, () => setOpen(false), { position: false });
  const anchor = layer.root;
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const [updatingCodex, setUpdatingCodex] = useState(false);
  const [updateWarning, setUpdateWarning] = useState('');
  const currentId = connectionId ?? activeProfile?.id ?? '';
  const { catalogs: magpieCatalogs, loading: loadingMagpie, warning: magpieWarning, refresh: refreshMagpie } = useMagpieCatalogs(open, agent, currentId, profiles);
  const currentCatalog = useMemo(() => catalogLoaded && source?.available === true ? { data: models, source } : undefined, [catalogLoaded, models, source]);
  const official = useOfficialCatalogs(true, agent, currentId, profiles, loggedInAgents, currentCatalog);
  const catalogs = useMemo(() => ({ ...magpieCatalogs, ...official.catalogs }), [magpieCatalogs, official.catalogs]);
  const accountsLoaded = accounts.agent === agent;
  const groups = useMemo(() => modelGroups({ value, models, catalogLoaded, catalogs, profiles, activeProfile, source, agent, connectionId, order, loggedInAgents, accountsLoaded }), [value, models, catalogLoaded, catalogs, profiles, activeProfile, source, agent, connectionId, order, loggedInAgents, accountsLoaded]);
  const magpieCurrent = profiles.find(profile => profile.agent === agent && profile.id === currentId && (profile.providerId === 'magpie' || profile.modelSource?.providerId === 'magpie' || source?.providerId === 'magpie'));
  const magpieGateway = useMagpieGateway(open && !!magpieCurrent, agent, magpieCurrent?.id);
  const currentGroup = groups.find(g => g.id === currentId || g.options.some(option => option.connectionId === currentId));
  const officialCodex = agent === 'codex' && !!currentGroup && (activeProfile?.officialAccount || currentId === '@official' || currentGroup.id === '@official');
  const selected = currentGroup?.options.find(m => m.selected);
  const selectedSource = selected?.source;
  const needsProvider = !groups.length && !loading && !checkingAccounts;
  const selectedName = selected?.displayName ?? (needsProvider ? '添加模型供应商' : (loading || checkingAccounts) && !currentGroup ? '正在检查连接…' : '选择模型');
  const visible = useMemo(() => filterModelGroups(groups, search, filter), [groups, search, filter]);
  const options = useMemo(() => visible.flatMap(group => group.options), [visible]);
  const currentPage = pageModels(options, page);
  const pageKeys = new Set(currentPage.items.map(option => option.key));
  // Verified connections remain browsable while their model catalogs load.
  const available = groups;
  const efforts = modelReasoningEfforts(agent, selected, selectedSource);
  const chosenEffort = efforts.includes(effort) ? effort : '';
  const effectiveEffort = chosenEffort || (efforts.includes(selected?.defaultReasoningEffort ?? '') ? selected?.defaultReasoningEffort : undefined);
  const effortName = selected && (efforts.length || effectiveEffort) ? effectiveEffort ? effortLabels[effectiveEffort] ?? effectiveEffort : '默认' : '';
  const speeds = useCodexSpeeds(!!officialCodex && desktop, currentGroup?.id ?? currentId, value || selected?.model || '', codexRevision);
  const declaredSpeeds = selected?.serviceTiers?.map(tier => tier.id === 'priority' ? 'fast' : tier.id) ?? [];
  const offeredSpeeds = ['fast', 'ultrafast'].filter(tier => declaredSpeeds.includes(tier) || speeds.tiers?.includes(tier) || serviceTier === tier);
  const showSpeeds = officialCodex && offeredSpeeds.length > 0 && !!setServiceTier;
  const speedLabels: Record<string, string> = { default: '标准', fast: 'Fast', ultrafast: '超快' };
  const contextVariants = selected && isKimiSource(selected.source) && agent === 'claude' && selected.rawIds.some(id => /^k3\[1m\]$/i.test(id)) && selected.rawIds.includes('k3') ? selected.rawIds : [];
  const latestRoute = useRef({ officialCodex, reload });
  latestRoute.current = { officialCodex, reload };
  const browsingCurrent = filter === null || filter === currentGroup?.id;
  const browsingOfficial = filter === null || filter === '@official' || !!profiles.find(profile => profile.id === filter && profile.agent === agent)?.officialAccount;
  const visibleUpdateWarning = browsingCurrent && officialCodex ? updateWarning : '';
  const relevantWarning = visibleUpdateWarning || (browsingCurrent && officialCodex ? speeds.warning : '')
    || (browsingOfficial ? accountWarning || official.warning : '')
    || (browsingCurrent ? catalogWarning : '')
    || (visible.some(group => group.source.providerId === 'magpie') ? magpieWarning : '')
    || (browsingCurrent && magpieCurrent && ['offline', 'unknown', 'unconfigured'].includes(magpieGateway ?? '') ? magpieGatewayText(magpieGateway) : '');

  useEffect(() => {
    // A temporary profile placeholder is not the route's actual model catalog.
    if (!busy && !loading && selected && models.some(model => model.model === selected.model) && modelReasoningKnown(agent, selected, selectedSource) && effort && !efforts.includes(effort)) setEffort('');
  }, [agent, busy, loading, selected?.model, models, effort, efforts.join(','), setEffort]);

  async function updateCodex() {
    setUpdatingCodex(true); setUpdateWarning('');
    try {
      await call('update_agent', { id: 'codex' });
      if (latestRoute.current.officialCodex) latestRoute.current.reload(true);
      official.refresh(); speeds.refresh(); setAccountRevision(value => value + 1);
    } catch (error) { setUpdateWarning(String(error)); }
    finally { setUpdatingCodex(false); }
  }
  function close(focus = false) { setOpen(false); if (focus) trigger.current?.focus({ preventScroll: true }); }
  useEffect(() => { if (busy) setOpen(false); }, [busy]);
  useEffect(() => {
    if (!desktop || !['claude', 'codex', 'gemini'].includes(agent)) return;
    let disposed = false;
    setAccountWarning('');
    const force = refreshedAccount.current !== accountRevision;
    refreshedAccount.current = accountRevision;
    void call<{ agent: string; connectionId?: string; loggedIn: boolean; error?: string }[]>('list_provider_accounts', { force, agent }).then(rows => {
      if (!disposed) setAccounts({ agent, ids: rows.filter(row => row.loggedIn).map(row => row.connectionId ?? row.agent) });
      if (!disposed) setAccountWarning(rows.filter(row => row.agent === agent && row.error).map(row => row.error).join('\n'));
    }).catch(error => { if (!disposed) { setAccounts({ agent, ids: [] }); setAccountWarning(`无法读取官方账号：${String(error)}`); } });
    return () => { disposed = true; };
  }, [agent, profiles, accountRevision]);
  useLayoutEffect(() => {
    if (!open) return;
    const host = anchor.current?.closest<HTMLElement>('.workspace,.side-chat');
    function position() {
      const parent = anchor.current?.getBoundingClientRect();
      const boundary = host?.getBoundingClientRect();
      const leftEdge = boundary && boundary.width > 0 ? Math.max(12, boundary.left + 12) : 12;
      const rightEdge = boundary && boundary.width > 0 ? Math.min(window.innerWidth - 12, boundary.right - 12) : window.innerWidth - 12;
      if (popover.current) popover.current.style.maxWidth = `${Math.max(0, rightEdge - leftEdge)}px`;
      const popup = popover.current?.getBoundingClientRect();
      const scroll = list.current?.getBoundingClientRect();
      if (parent && popup && scroll) {
        setLeft(Math.max(leftEdge, Math.min(parent.left, rightEdge - popup.width)) - parent.left);
        const controls = popup.height - scroll.height;
        setListHeight(Math.max(65, Math.min(264, parent.top - 22 - controls)));
      }
    }
    position(); window.addEventListener('resize', position);
    const observer = new ResizeObserver(position); if (popover.current) observer.observe(popover.current);
    if (host) observer.observe(host);
    return () => { window.removeEventListener('resize', position); observer.disconnect(); };
  }, [open, models, profiles, search, filter, currentPage.page, contextVariants.length, efforts.length, hasConversation]);
  function focusModel(key: string) {
    const button = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button.model-option') ?? []).find(button => button.dataset.modelKey === key);
    if (!button || !list.current) return;
    button.focus({ preventScroll: true });
    const row = button.getBoundingClientRect(), area = list.current.getBoundingClientRect();
    if (row.top < area.top) list.current.scrollTop += row.top - area.top;
    else if (row.bottom > area.bottom) list.current.scrollTop += row.bottom - area.bottom;
  }
  useLayoutEffect(() => {
    if (list.current) list.current.scrollTop = 0;
    if (open && pendingFocus.current) { focusModel(pendingFocus.current); pendingFocus.current = null; }
  }, [open, currentPage.page, options]);
  useLayoutEffect(() => {
    // Autofocus runs before the popup's viewport position is measured and can
    // scroll the entire chat to reveal its still-offscreen search field.
    if (open) searchInput.current?.focus({ preventScroll: true });
  }, [open]);

  async function select(option: ModelOption) {
    close(true);
    await choose(option);
  }
  function move(event: React.KeyboardEvent) {
    if (event.key === 'Enter' && event.target === searchInput.current && !event.nativeEvent.isComposing) { event.preventDefault(); const options = visible.flatMap(g => g.options); const option = options.find(o => o.selected) ?? options[0]; if (option) void select(option); return; }
    if (event.target !== searchInput.current && !list.current?.contains(event.target as Node)) return;
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    if (event.target === searchInput.current && ['Home', 'End'].includes(event.key)) return;
    if (!options.length) return;
    event.preventDefault();
    const key = (document.activeElement as HTMLElement | null)?.dataset.modelKey;
    const current = options.findIndex(option => option.key === key);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1 : event.key === 'ArrowDown' ? (current + 1) % options.length : (current <= 0 ? options.length : current) - 1;
    const nextPage = Math.floor(next / MODEL_PAGE_SIZE);
    if (nextPage === currentPage.page) focusModel(options[next].key);
    else { pendingFocus.current = options[next].key; setPage(nextPage); }
  }
  return <div className="composer-menu-anchor" ref={anchor}>
    <button ref={trigger} type="button" className="composer-chip model-chip" aria-label={needsProvider ? '添加模型供应商' : '选择模型'} aria-haspopup={needsProvider ? undefined : 'dialog'} aria-expanded={open} disabled={busy}
      title={[selectedName, selectedSource && sourceLabel(selectedSource), selected && `模型 ID：${selected.model}`, effortName && `推理强度：${effortName}${!effort && effectiveEffort ? '（默认）' : ''}`].filter(Boolean).join('\n')}
      onClick={() => { if (needsProvider) { manage(); return; } setOpen(v => !v); setSearch(''); setFilter(null); pendingFocus.current = null; setPage(Math.floor(Math.max(0, groups.flatMap(group => group.options).findIndex(option => option.selected)) / MODEL_PAGE_SIZE)); if (!open && desktop && !loading && (magpieCurrent || !models.length)) reload(magpieCurrent ? true : undefined); }}>
      {selectedSource?.mark ? <span className="model-provider-mark" aria-hidden="true"><ProviderIcon provider={selectedSource.providerId} name={selectedSource.connectionName || selectedSource.providerName} mark={selectedSource.mark}/></span> : null}
      <span className="model-chip-name">{selectedName}</span>
      {effortName ? <span className="model-chip-effort" title={`推理强度：${effortName}`}>{effortName}</span> : null}
      {selectedSource?.providerName ? <span className="model-chip-provider">{selectedSource.providerName}</span> : null}
      {needsProvider ? <Plus size={12}/> : <ChevronDown size={11} />}
    </button>
    {open ? <div ref={popover} className="composer-popover model-popover" style={{ left }} role="dialog" aria-label="模型选择" onKeyDown={move}>
      <div className="model-menu-heading"><h2>选择模型</h2><span title={selectedName}>{selectedName}</span><button type="button" className="icon-button" aria-label="关闭模型选择" onClick={() => close(true)}><X size={15}/></button></div>
      <div className="model-picker-search search-field">
        <Search size={15}/><input ref={searchInput} aria-label="搜索模型菜单" placeholder="搜索模型或供应商" value={search} onChange={e => { setSearch(e.target.value); setPage(0); pendingFocus.current = null; }} />
        <button type="button" className="icon-button" aria-label="刷新模型" title="刷新模型" disabled={loading || loadingMagpie || official.loading || busy || !desktop} onClick={() => { reload(true); refreshMagpie(); official.refresh(); speeds.refresh(); setAccountRevision(value => value + 1); }}>{loading || loadingMagpie || official.loading ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}</button>
      </div>
      {available.length > 1 ? <div className="model-provider-filters" role="group" aria-label="按供应商筛选">
        <button type="button" aria-pressed={filter === null} onClick={() => { setFilter(null); setPage(0); }}>全部</button>
        {available.map(g => <button type="button" key={g.id} aria-pressed={filter === g.id} title={sourceLabel(g.source)} onClick={() => { setFilter(g.id); setPage(0); }}><ProviderIcon provider={g.source.providerId} name={connectionLabel(g.source)} mark={g.source.mark}/><span>{connectionLabel(g.source)}</span></button>)}
      </div> : null}
      <div ref={list} className="model-menu-scroll" style={{ maxHeight: listHeight }} role="menu" aria-label="可用模型">
        {visible.filter(group => !group.options.length || group.options.some(option => pageKeys.has(option.key))).map(group => <div className="model-source-group" key={group.id}>
          <div className="model-group-heading" title={sourceLabel(group.source)}><span><span className="model-group-mark" aria-hidden="true"><ProviderIcon provider={group.source.providerId} name={connectionLabel(group.source)} mark={group.source.mark}/></span><span>{connectionLabel(group.source)}</span></span></div>
          {!group.options.length ? <div className="model-menu-empty">{loading || official.loading || loadingMagpie ? '正在加载模型…' : relevantWarning ? '暂时无法读取模型' : '暂无模型'}</div> : null}
          {group.options.filter(option => pageKeys.has(option.key)).map(option => <button type="button" key={option.key} data-model-key={option.key} className={`model-option ${option.selected ? 'selected' : ''}`} role="menuitemradio" aria-checked={option.selected} disabled={busy}
            title={`模型 ID：${option.rawIds.join(' / ')}${option.context === '1M' ? '\n已配置 Claude Code 1M 上下文参数' : ''}`}
            onClick={() => void select(option)}>
            <span className="model-option-copy"><strong>{option.displayName}</strong>{option.context ? <span className="model-context-badge">{option.context}</span> : null}</span>
            <span className="model-option-check">{option.selected ? <Check size={15} /> : null}</span>
          </button>)}
        </div>)}
        {!visible.length ? <div className="model-menu-empty">{checkingAccounts ? '正在检查账号…' : loading || official.loading || loadingMagpie ? '正在加载模型…' : relevantWarning ? '暂时无法读取模型' : search || filter !== null ? '没有匹配的模型' : '尚未配置模型'}</div> : null}
      </div>
      <ModelPagination count={options.length} page={currentPage.page} change={setPage} disabled={busy}/>
      <div className="model-menu-footer">
        {selected && contextVariants.length > 1 ? <ModelSetting label="上下文" value={selected.model} options={contextVariants.map(id => ({ value: id, label: /\[1m\]$/i.test(id) ? '1M' : '默认' }))} choose={model => void choose({ ...selected, model, defaultReasoningEffort: effort })}/> : null}
        {efforts.length ? agent === 'codex' ? <CodexReasoningSlider modelName={selectedName} efforts={efforts} value={chosenEffort} defaultEffort={selected?.defaultReasoningEffort} choose={setEffort} showModels={() => searchInput.current?.focus({ preventScroll: true })} disabled={busy}/>
          : <ModelSetting label="推理强度" value={chosenEffort} options={[{ value: '', label: '默认' }, ...efforts.map(value => ({ value, label: effortLabels[value] ?? value, title: value }))]} choose={setEffort}/> : null}
        {showSpeeds && setServiceTier ? <ModelSetting label="速度" value={serviceTier || 'default'} options={['default', ...offeredSpeeds].map(value => ({ value, label: speedLabels[value], disabled: updatingCodex || value !== 'default' && !speeds.tiers?.includes(value), title: value === 'default' ? undefined : speeds.tiers?.includes(value) ? '使用更多额度' : speeds.loading ? '正在读取速度' : '当前 Codex 或账号未提供此速度' }))} choose={setServiceTier}/> : null}
        {browsingCurrent && officialCodex && !visibleUpdateWarning && (updatingCodex || speeds.modelUnavailable) ? <div className="model-menu-status" role="status"><span>{updatingCodex ? '正在更新 Codex…' : 'Codex 未提供此模型'}</span><button type="button" disabled={busy || updatingCodex} onClick={() => void updateCodex()}>更新 Codex</button></div>
          : relevantWarning ? <details className="model-menu-error"><summary>{visibleUpdateWarning ? '更新失败' : '读取失败'}<span>详情</span></summary><p>{relevantWarning}</p></details> : null}
        <div className="model-menu-actions">
          <button type="button" onClick={() => { close(); manage(); }}><SlidersHorizontal size={14}/>模型设置</button>
          {visibleUpdateWarning ? <button type="button" disabled={busy || updatingCodex} onClick={() => void updateCodex()}>重试更新</button> : null}
        </div>
      </div>
    </div> : null}
  </div>;
}
