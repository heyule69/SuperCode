import { useEffect, useRef, useState } from 'react';
import { BookOpen, Check, ChevronDown, ChevronRight, FolderOpen, Globe, Plus, Puzzle, RefreshCw, Search, Server, X } from 'lucide-react';
import { call, desktop } from './api';
import { AgentIcon } from './AgentIcon';
import { loadLocalSkills } from './skills';
import { matchesExtension, parseToolArgs, pluginParentEnabled, type ExtensionCatalog, type ExtensionPlugin, type ToolServer } from './extensions';

const empty: ExtensionCatalog = { plugins: [], mcp: [], warnings: [] };
export default function ExtensionsSettings({ projectId, busy }: { projectId: string; busy: boolean }) {
  const [catalog, setCatalog] = useState(empty);
  const [servers, setServers] = useState<ToolServer[]>([]);
  const [deps, setDeps] = useState<{ npx?: string; uvx?: string }>({});
  const [tab, setTab] = useState<'plugins' | 'mcp'>('plugins');
  const [agent, setAgent] = useState('all');
  const [search, setSearch] = useState('');
  const [browse, setBrowse] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [expanded, setExpanded] = useState('');
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<ToolServer>();
  const [argsText, setArgsText] = useState('');
  const [importAgent, setImportAgent] = useState('codex');
  const [importing, setImporting] = useState(false);
  const [status, setStatus] = useState<{ name: string; tools?: Record<string, unknown>; error?: string }[]>();
  const generation = useRef(0);
  const lock = useRef(false);
  async function reload(includeCache = browse) {
    const revision = ++generation.current;
    const [extensions, tools] = await Promise.all([
      call<ExtensionCatalog>('list_extensions', { projectId: projectId || null, browse: includeCache }),
      call<{ servers: ToolServer[]; npx?: string; uvx?: string }>('list_tool_servers'),
    ]);
    if (revision !== generation.current) return;
    setCatalog(extensions); setServers(tools.servers); setDeps(tools);
  }
  async function run(action: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true; setWorking(true); setError(''); setMessage('');
    try { await action(); } catch (e) { setError(String(e)); }
    finally { lock.current = false; setWorking(false); }
  }
  useEffect(() => {
    if (!desktop) return;
    let cancelled = false;
    setWorking(true); setError('');
    void reload(false).catch(e => { if (!cancelled) setError(String(e)); }).finally(() => { if (!cancelled) setWorking(false); });
    return () => { cancelled = true; generation.current++; };
  }, [projectId]);
  async function toggle(id: string, enabled: boolean) {
    await call('set_extension_enabled', { projectId: projectId || null, id, enabled });
    await reload(); setStatus(undefined);
    await loadLocalSkills(projectId, true);
    setMessage('已保存，下一次任务生效');
  }
  async function saveTools(next: ToolServer[]) {
    await call('save_tool_servers', { servers: next }); setServers(next); setStatus(undefined);
    setMessage('已保存，下一次任务生效');
  }
  function editor(server: ToolServer) { setEditing(server); setArgsText(server.args.join('\n')); setAdding(false); }
  function preset(kind: 'custom' | 'browser' | 'computer') {
    const existing = servers.find(s => s.kind === kind && kind !== 'custom');
    if (existing) { editor(existing); return; }
    editor(kind === 'browser' ? { id: 'browser', name: '浏览器 · Playwright', kind, command: deps.npx ?? 'npx', args: ['-y', '@playwright/mcp@0.0.83', '--browser', 'msedge', '--isolated'], url: null, enabled: false }
      : kind === 'computer' ? { id: 'computer', name: '电脑 · Windows MCP', kind, command: deps.uvx ?? 'uvx', args: ['windows-mcp@0.7.5', 'serve'], url: null, enabled: false }
      : { id: `tool_${crypto.randomUUID().replace(/-/g, '')}`, name: '', kind, command: '', args: [], url: null, enabled: false });
    setTab('mcp');
  }
  async function importPlugin(plugin?: ExtensionPlugin) {
    const path = plugin?.path ?? await (await import('@tauri-apps/plugin-dialog')).open({ directory: true, title: '选择包含 plugin.json 的插件文件夹' });
    if (!path || Array.isArray(path)) return;
    await call('import_extension', { path, agent: plugin?.agent ?? importAgent });
    await reload(); await loadLocalSkills(projectId, true); setImporting(false);
    setMessage('插件已添加，下一次任务生效');
  }
  const plugins = catalog.plugins.filter(p => (browse || p.installed) && matchesExtension(p, agent, search));
  const nativeMcp = catalog.mcp.filter(p => matchesExtension(p, agent, search));
  const custom = servers.filter(s => matchesExtension({ ...s, agent: 'both', source: 'SuperCode' }, agent, search));
  const disabled = working || busy || !desktop;
  return <div className="extensions-settings">
    <div className="settings-page-heading"><div><h2>插件与 MCP</h2><p>管理本机插件与工具连接</p></div><div>
      <button className="quiet-button" disabled={working || !desktop} onClick={() => void run(async () => { setBrowse(!browse); setTab('plugins'); setSearch(''); await reload(!browse); })}><BookOpen size={14}/>{browse ? '已添加' : '浏览目录'}</button>
      <button className="primary-button" disabled={disabled} onClick={() => { setAdding(!adding); setImporting(false); }} aria-expanded={adding}>添加<ChevronDown size={13}/></button>
    </div></div>
    {adding ? <div className="extensions-add" aria-label="添加扩展"><button onClick={() => { setImporting(true); setAdding(false); }}><Puzzle size={16}/>导入插件文件夹</button><button onClick={() => preset('custom')}><Server size={16}/>添加 MCP 服务</button><button onClick={() => preset('browser')}><Globe size={16}/>浏览器 · Playwright</button><button onClick={() => preset('computer')}><FolderOpen size={16}/>电脑 · Windows MCP</button></div> : null}
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    {message ? <p className="provider-feedback" role="status"><Check size={14}/>{message}</p> : null}
    {!desktop ? <p className="muted">桌面版会自动读取本机已安装的插件和 MCP 配置。</p> : null}
    <div className="extensions-toolbar"><div className="extensions-tabs" aria-label="扩展类型">
      <button className={tab === 'plugins' ? 'active' : ''} aria-pressed={tab === 'plugins'} onClick={() => { setTab('plugins'); setSearch(''); }}>插件 <span>{catalog.plugins.filter(p => p.installed).length}</span></button>
      <button className={tab === 'mcp' ? 'active' : ''} aria-pressed={tab === 'mcp'} onClick={() => { setTab('mcp'); setSearch(''); }}>MCP <span>{catalog.mcp.length + servers.length}</span></button>
    </div><div className="extensions-search"><Search size={15}/><input aria-label="搜索插件和 MCP" placeholder={tab === 'plugins' ? '搜索插件' : '搜索 MCP'} value={search} onChange={e => setSearch(e.target.value)}/><button className="icon-button" aria-label="刷新扩展" disabled={working || !desktop} onClick={() => void run(() => reload())}><RefreshCw size={14}/></button></div></div>
    <div className="extensions-filters" aria-label="按 Agent 筛选">{[['all', '全部'], ['codex', 'Codex'], ['claude', 'Claude Code']].map(([id, name]) => <button key={id} className={agent === id ? 'active' : ''} aria-pressed={agent === id} onClick={() => setAgent(id)}>{id !== 'all' ? <AgentIcon agent={id}/> : null}{name}</button>)}</div>
    {importing ? <section className="extensions-editor"><div className="settings-section-heading"><h3>导入插件</h3><button className="icon-button" aria-label="关闭导入插件" onClick={() => setImporting(false)}><X size={15}/></button></div><p className="muted">选择包含 plugin.json 的文件夹，保留插件原有文件。</p><label>使用的 Agent<select value={importAgent} onChange={e => setImportAgent(e.target.value)}><option value="codex">Codex</option><option value="claude">Claude Code</option></select></label><button className="primary-button" disabled={disabled} onClick={() => void run(() => importPlugin())}><FolderOpen size={14}/>选择文件夹</button></section> : null}
    {editing ? <form className="extensions-editor" onSubmit={e => { e.preventDefault(); void run(async () => { const next = { ...editing, name: editing.name.trim(), args: parseToolArgs(argsText), url: editing.url?.trim() || null }; await saveTools([...servers.filter(s => s.id !== next.id), next]); setEditing(undefined); }); }}>
      <div className="settings-section-heading"><h3>{servers.some(s => s.id === editing.id) ? '编辑 MCP' : '添加 MCP'}</h3><button type="button" className="icon-button" aria-label="关闭 MCP 配置" onClick={() => setEditing(undefined)}><X size={15}/></button></div>
      <div className="preference-grid"><label>名称<input required value={editing.name} onChange={e => setEditing({ ...editing, name: e.target.value })}/></label><label>传输<select value={editing.url === null ? 'stdio' : 'http'} onChange={e => setEditing({ ...editing, url: e.target.value === 'http' ? '' : null })}><option value="stdio">本机程序 / stdio</option><option value="http">HTTP MCP</option></select></label></div>
      {editing.url !== null ? <label>地址<input required placeholder="https://example.com/mcp" value={editing.url} onChange={e => setEditing({ ...editing, url: e.target.value })}/></label> : <><label>程序<input required value={editing.command} onChange={e => setEditing({ ...editing, command: e.target.value })}/></label><label>参数（每行一项或 JSON 数组）<textarea rows={3} value={argsText} onChange={e => setArgsText(e.target.value)}/></label></>}
      <div className="settings-save-row"><label className="preference-toggle"><span>启用</span><input type="checkbox" role="switch" checked={editing.enabled} onChange={e => setEditing({ ...editing, enabled: e.target.checked })}/></label><span>Codex 与 Claude Code 共用</span>{servers.some(s => s.id === editing.id) ? <button className="quiet-button" type="button" disabled={disabled} onClick={() => void run(async () => { await saveTools(servers.filter(s => s.id !== editing.id)); setEditing(undefined); })}>删除</button> : null}<button className="primary-button" disabled={disabled}>保存</button></div>
    </form> : null}
    {tab === 'plugins' ? <><div className="extensions-section-heading"><span>{browse ? '本机插件目录' : '已添加的插件'}</span><span>{plugins.length} 个</span></div><div className="extensions-list">
      {plugins.map(p => <div className="extension-item" key={p.id}><div className="extension-item-main"><div className="extension-art">{p.icon ? <img src={p.icon} alt=""/> : <Puzzle size={21}/>}</div><button className="extension-description" onClick={() => setExpanded(expanded === p.id ? '' : p.id)} aria-expanded={expanded === p.id}><strong>{p.name}</strong><p>{p.description || p.components.join(' · ') || '本机插件'}</p><small><AgentIcon agent={p.agent}/>{p.agent === 'claude' ? 'Claude Code' : 'Codex'}{p.components.length ? ` · ${p.components.join('、')}` : ''}</small></button>
        {p.limitation ? <span className="extension-availability" title={p.limitation}>{p.limitation.includes('客户端') ? '需官方客户端' : '暂不可用'}</span> : p.installed ? <label className="preference-toggle extension-switch"><input type="checkbox" role="switch" aria-label={`启用插件 ${p.name}`} checked={p.enabled} disabled={disabled} onChange={e => void run(() => toggle(p.id, e.target.checked))}/></label> : <button className="quiet-button" disabled={disabled} onClick={() => void run(() => importPlugin(p))}><Plus size={13}/>添加</button>}
        <button className="icon-button" aria-label={`查看 ${p.name} 详情`} aria-expanded={expanded === p.id} onClick={() => setExpanded(expanded === p.id ? '' : p.id)}>{expanded === p.id ? <ChevronDown size={14}/> : <ChevronRight size={14}/>}</button></div>
        {expanded === p.id ? <div className="extension-details"><p>{p.description}</p>{p.limitation ? <p>{p.limitation}</p> : null}<dl><dt>来源</dt><dd>{p.source}</dd>{p.version ? <><dt>版本</dt><dd>{p.version}</dd></> : null}<dt>文件夹</dt><dd>{p.path}</dd></dl><button className="quiet-button" disabled={!desktop || working} onClick={() => void run(async () => { await call('open_extension_folder', { id: p.id, projectId: projectId || null }); })}><FolderOpen size={14}/>打开文件夹</button></div> : null}</div>)}
    </div>{!plugins.length && !working ? <div className="extensions-empty"><Puzzle size={25}/><strong>{search ? '没有匹配的插件' : '尚未发现本机插件'}</strong><p>{search ? '试试其他名称，或切换 Agent。' : '导入插件文件夹，或安装后刷新列表。'}</p></div> : null}
      <div className="extensions-catalog-links"><span>更多插件</span><button className="quiet-button" disabled={!desktop} onClick={() => void run(async () => { await call('open_external_link', { url: 'https://chatgpt.com/plugins' }); })}>Codex 官方目录 ↗</button><button className="quiet-button" disabled={!desktop} onClick={() => void run(async () => { await call('open_external_link', { url: 'https://code.claude.com/docs/en/discover-plugins' }); })}>Claude Code 插件 ↗</button></div>
    </> : <><div className="extensions-section-heading"><span>工具连接</span><button className="quiet-button" disabled={disabled} onClick={() => preset('custom')}><Plus size={13}/>添加 MCP</button></div><div className="extensions-list">
      {nativeMcp.map(m => { const parent = pluginParentEnabled(m, catalog.plugins); const connection = m.agent === 'codex' ? status?.find(s => s.name === m.name || s.name.endsWith(`/${m.name}`)) : undefined; return <div className="extension-item" key={m.id}><div className="extension-item-main"><div className="extension-art"><Server size={20}/></div><div className="extension-description"><strong>{m.name}</strong><p>{m.source} · {m.transport}{m.endpoint ? ` · ${m.endpoint}` : ''}</p><small><AgentIcon agent={m.agent}/>{m.agent === 'claude' ? 'Claude Code' : 'Codex'}{m.limitation ? ` · ${m.limitation}` : !parent ? ' · 先启用所属插件' : connection ? ` · ${connection.error ? '连接失败' : `${Object.keys(connection.tools ?? {}).length} 个工具`}` : ` · ${m.enabled ? '已启用' : '已关闭'}`}</small></div><label className="preference-toggle extension-switch"><input type="checkbox" role="switch" aria-label={`启用 MCP ${m.name} ${m.agent}`} checked={m.enabled} disabled={disabled || !!m.limitation || !parent} onChange={e => void run(() => toggle(m.id, e.target.checked))}/></label></div></div>; })}
      {custom.map(s => <div className="extension-item" key={s.id}><div className="extension-item-main"><div className="extension-art"><Server size={20}/></div><button className="extension-description" onClick={() => editor(s)}><strong>{s.name}</strong><p>SuperCode · {s.url ? 'HTTP' : 'stdio'}</p><small>Codex 与 Claude Code · {s.enabled ? '已启用' : '已关闭'}</small></button><label className="preference-toggle extension-switch"><input type="checkbox" role="switch" aria-label={`启用 MCP ${s.name}`} checked={s.enabled} disabled={disabled} onChange={e => void run(() => saveTools(servers.map(v => v.id === s.id ? { ...v, enabled: e.target.checked } : v)))}/></label><button className="icon-button" aria-label={`编辑 ${s.name}`} onClick={() => editor(s)}><ChevronRight size={14}/></button></div></div>)}
    </div>{!nativeMcp.length && !custom.length && !working ? <div className="extensions-empty"><Server size={25}/><strong>{search ? '没有匹配的 MCP' : '尚未添加 MCP'}</strong><p>添加 HTTP 服务或本机程序后，Agent 可以使用对应工具。</p></div> : null}
      <div className="extensions-catalog-links"><span>工具在执行任务时按需连接</span><button className="quiet-button" disabled={disabled} onClick={() => void run(async () => { const result = await call<{ data?: { name: string; tools?: Record<string, unknown>; error?: string }[] }>('tool_server_status'); setStatus(result.data ?? []); setMessage('已读取 Codex MCP 运行状态'); })}>检查 Codex 连接</button></div>
    </>}
    {working ? <p className="muted" role="status">正在读取扩展…</p> : null}
    {catalog.warnings.length ? <details className="extensions-warnings"><summary>{catalog.warnings.length} 条读取提示</summary>{catalog.warnings.map((warning, index) => <p key={index}>{warning}</p>)}</details> : null}
  </div>;
}
