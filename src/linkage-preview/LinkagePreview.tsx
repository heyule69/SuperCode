import { useState, type FormEvent } from 'react';
import { ArrowDown, ArrowUp, Check, CheckCheck, ChevronDown, ChevronRight, CircleAlert, CircleHelp, FileCode2, Folder, GitBranch, Link2, LoaderCircle, MessageSquare, Moon, PanelLeft, PanelRightClose, PanelRightOpen, Plus, Search, SlidersHorizontal, Square, SquarePen, Sun, Terminal, X } from 'lucide-react';
import { AgentIcon } from '../AgentIcon';
import { AgentMenu } from '../AgentMenu';
import { DesktopTitleBar } from '../DesktopTitleBar';
import { NewChat } from '../NewChat';
import { firstLinkageChoice, type LinkageConfiguration } from '../LinkageSetup';
import { LinkageModelPicker } from '../LinkageModelPicker';
import { LinkageDialog } from '../LinkageDialog';
import { PermissionMenu } from '../PermissionMenu';
import type { PermissionMode } from '../types';
import { catalogs, initialConfiguration, selectionName } from './catalog';

type Stage = 'setup' | 'running' | 'waiting' | 'completed' | 'failed' | 'stopped';
type CatalogState = 'ready' | 'loading' | 'empty' | 'error';
const stageNames: Record<Stage, string> = { setup: '新聊天', running: '正在执行', waiting: '等待确认', completed: '已完成', failed: '执行失败', stopped: '已停止' };
const project = { id: 'preview', name: 'SuperCode', path: '示例项目' };
const sampleRequest = '把设置页的保存按钮放到右上角，保持现在的样式。';

function initialStage(): Stage {
  const value = new URLSearchParams(location.search).get('state');
  return value && Object.hasOwn(stageNames, value) ? value as Stage : 'setup';
}

export function LinkagePreview() {
  const [configuration, setConfiguration] = useState<LinkageConfiguration>(() => ({ ...initialConfiguration, mode: initialStage() === 'setup' ? 'single' : 'linked' }));
  const [stage, setStage] = useState<Stage>(initialStage);
  const [catalogState, setCatalogState] = useState<CatalogState>('ready');
  const [sidebar, setSidebar] = useState(true);
  const [panelOpen, setPanelOpen] = useState(true);
  const [tab, setTab] = useState<'activity' | 'changes'>('activity');
  const [draft, setDraft] = useState('');
  const [request, setRequest] = useState(sampleRequest);
  const [showOriginal, setShowOriginal] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [search, setSearch] = useState(false);
  const [searchText, setSearchText] = useState('');
  const [selectedProject, setSelectedProject] = useState<typeof project | undefined>(project);
  const [notice, setNotice] = useState('');
  const [permission, setPermission] = useState<PermissionMode>('ask');
  const [theme, setTheme] = useState(document.documentElement.dataset.theme ?? 'light');
  const controller = selectionName(configuration.controller);
  const executor = selectionName(configuration.executor);
  const controllerCatalog = catalogs.find(item => item.agent.id === configuration.controller.agentId);
  const linked = configuration.mode === 'linked';
  const panel = stage !== 'setup' && linked && panelOpen;
  const busy = stage === 'running' || stage === 'waiting';
  const displayCatalogs = catalogState === 'ready' ? catalogs : catalogs.map(item => item.agent.id !== configuration.controller.agentId ? item : {
    ...item, connections: catalogState === 'empty' ? [] : item.connections,
    loading: catalogState === 'loading', error: catalogState === 'error' ? '模型连接暂不可用' : undefined,
  });

  function newChat() { setStage('setup'); setDraft(''); setSearch(false); setShowSettings(false); setShowOriginal(false); }
  function previewConversation() {
    setRequest(draft.trim() || sampleRequest); setDraft(''); setStage('running'); setPanelOpen(true); setShowSettings(false);
  }
  function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    previewConversation();
  }
  function changeTheme() {
    const next = theme === 'light' || theme === 'sand' ? 'dark' : 'light';
    document.documentElement.dataset.theme = next; setTheme(next);
  }
  function example(next: Stage) { setCatalogState('ready'); setConfiguration(value => ({ ...value, mode: 'linked' })); setStage(next); setSearch(false); setPanelOpen(true); }
  const step = stage === 'completed' ? 3 : 1;
  const progressText = stage === 'completed' ? '执行结果已交给主控' : stage === 'waiting' ? '执行者正在等待你的选择'
    : stage === 'failed' ? '连接中断，任务未完成' : stage === 'stopped' ? '联动任务已停止' : '正在调整按钮布局';
  const assistantText = stage === 'completed' ? '保存按钮已移到设置页右上角，沿用原来的颜色和圆角。保存方式没有变化，窄窗口下也保留了操作空间。'
    : stage === 'waiting' ? '执行到保存行为时，需要你决定：保存后继续留在设置页，还是返回聊天？选好后，我会把你的要求交给执行 Agent。'
    : stage === 'failed' ? `执行过程中，${executor.agent} 的模型连接中断了。当前任务还没完成，执行记录已经保留。你可以检查连接后继续。`
    : stage === 'stopped' ? '已经停止这次联动。执行记录保留在右侧，你可以补充要求后再继续。'
    : linked ? `我会保留现在的视觉样式，让 ${executor.agent} 调整保存按钮的位置，完成后把改动和验证结果整理给你。`
    : '我会保留现在的视觉样式，调整保存按钮的位置，并检查窄窗口下的显示。';

  return <div className="desktop-shell linkage-preview">
    <DesktopTitleBar back={newChat} forward={() => example('running')} canBack={stage !== 'setup'} canForward={stage === 'setup'} sidebar={sidebar}
      toggleSidebar={() => setSidebar(v => !v)} blocked={showSettings} report={console.error} menus={[
        { label: '文件', items: [{ label: '新聊天', run: newChat }] },
        { label: '查看', items: [{ label: '切换深浅主题', run: changeTheme }, { label: '执行详情', run: () => { example('running'); setPanelOpen(true); } }] },
      ]}/>
    <div className="linkage-preview-strip">
      <span><span className="linkage-preview-dot"/>交互预览<span className="linkage-preview-explanation">示例数据</span></span>
      <div><label>{stage === 'setup' ? '配置状态' : '示例状态'}<select aria-label={stage === 'setup' ? '配置状态' : '示例状态'}
        value={stage === 'setup' ? catalogState : stage} onChange={event => stage === 'setup' ? setCatalogState(event.target.value as CatalogState) : setStage(event.target.value as Stage)}>
        {stage === 'setup' ? <><option value="ready">连接可用</option><option value="loading">加载中</option><option value="empty">未配置连接</option><option value="error">连接失败</option></>
          : Object.entries(stageNames).filter(([id]) => id !== 'setup').map(([id, name]) => <option key={id} value={id}>{name}</option>)}
      </select></label><button type="button" onClick={changeTheme} aria-label="切换深浅主题" title="切换深浅主题">{theme === 'light' || theme === 'sand' ? <Moon size={14}/> : <Sun size={14}/>}</button></div>
    </div>
    <div className="desktop-workarea">
      <nav className="activity-rail" aria-label="工作台导航">
        <button type="button" className="linkage-rail-logo" aria-label="返回新聊天" onClick={newChat}><img src="/app-icon.png" alt=""/></button>
        <button type="button" className="selected" aria-label="查看示例对话" onClick={() => example('running')}><MessageSquare size={19}/></button>
        <button type="button" aria-label="新建聊天" onClick={newChat}><SquarePen size={19}/></button>
        <button type="button" aria-label="搜索示例聊天" onClick={() => { setSidebar(true); setSearch(v => !v); }}><Search size={19}/></button>
        <span className="activity-rail-spacer"/>
        <button type="button" aria-label="联动配置" onClick={() => { if (stage === 'setup') setShowSettings(false); else setShowSettings(v => !v); }}><SlidersHorizontal size={18}/></button>
      </nav>
      <div className={`app linkage-preview-app ${panel ? '' : 'context-hidden'} ${sidebar ? '' : 'sidebar-hidden'}`} inert={showSettings}>
        <aside className="sidebar" hidden={!sidebar} aria-label="聊天侧栏">
          <div className="brand"><span>SuperCode</span><button type="button" className="icon-button brand-search" aria-label="搜索聊天" onClick={() => setSearch(v => !v)}><Search size={16}/></button></div>
          <button type="button" className="new-chat" onClick={newChat}><SquarePen size={15}/>新聊天</button>
          {search ? <label className="linkage-sidebar-search"><Search size={14}/><input autoFocus aria-label="搜索示例" placeholder="搜索聊天…" value={searchText} onChange={event => setSearchText(event.target.value)}/><button type="button" aria-label="关闭搜索" onClick={() => { setSearch(false); setSearchText(''); }}><X size={13}/></button></label> : null}
          <div className="linkage-sidebar-content">
            <div className="linkage-sidebar-heading"><span>项目</span><button type="button" aria-label="打开项目配置" onClick={newChat}><Plus size={14}/></button></div>
            <button type="button" className="project-row" onClick={newChat}><ChevronDown size={12}/><Folder size={15}/><span>SuperCode</span></button>
            <div className="linkage-sidebar-chats">
              {[
                { title: '调整设置页的保存按钮', stage: 'running' as const },
                { title: '查看已完成的联动', stage: 'completed' as const },
                { title: '查看等待确认的联动', stage: 'waiting' as const },
              ].filter(item => item.title.includes(searchText)).map(item => <button type="button" key={item.title} className={stage === item.stage ? 'selected' : ''}
                onClick={() => { setRequest(sampleRequest); example(item.stage); }}><Link2 size={13}/><span>{item.title}</span></button>)}
              {searchText && !['调整设置页的保存按钮', '查看已完成的联动', '查看等待确认的联动'].some(title => title.includes(searchText)) ? <span className="linkage-search-empty">没有匹配的示例聊天</span> : null}
            </div>
            <div className="linkage-sidebar-heading linkage-recent-heading"><span>最近聊天</span><span>今天</span></div>
            <button type="button" className="linkage-recent-chat" onClick={newChat}><Link2 size={14}/><span>尝试 Agent 联动</span><span className="linkage-sidebar-tag">新</span></button>
          </div>
          <div className="linkage-sidebar-footer"><span className="linkage-ready-dot"/>本地工作区<small>设计预览</small></div>
        </aside>
        <div className="conversation-surface">
          <main className={`workspace${stage === 'setup' ? ' linkage-original-home' : ''}`}>
            <header className="workspace-header">
              <div className="breadcrumbs">{!sidebar ? <button type="button" className="icon-button" aria-label="展开侧栏" onClick={() => setSidebar(true)}><PanelLeft size={16}/></button> : null}<Folder size={14}/><span>{selectedProject?.name ?? '工作区'}</span><ChevronRight size={12}/><strong title={request}>{stage === 'setup' ? '新聊天' : request}</strong></div>
              <div className="header-actions">{stage !== 'setup' ? <>
                <button type="button" className={`linkage-header-mode${showSettings ? ' selected' : ''}`} onClick={() => setShowSettings(v => !v)} aria-expanded={showSettings}><Link2 size={13}/>{linked ? 'Agent 联动' : '普通聊天'}<ChevronDown size={12}/></button>
                <button type="button" className="icon-button" aria-label={panel ? '收起执行详情' : '打开执行详情'} onClick={() => setPanelOpen(v => !v)}>{panel ? <PanelRightClose size={17}/> : <PanelRightOpen size={17}/>}</button>
              </> : <button type="button" className="icon-button" aria-label="预览工作空间" onClick={() => setNotice('工作空间继续使用现有的文件与代码改动面板。')}><PanelRightOpen size={17}/></button>}</div>
            </header>
            <div className="linkage-preview-scroll">
              {stage === 'setup' ? <NewChat project={selectedProject} projects={[project]} selectProject={setSelectedProject} addProject={() => setNotice('这里沿用现有的项目选择与添加入口。当前预览使用示例项目。')}/> : <>
                <div className="linkage-preview-messages">
                  <div className="linkage-user-message">{request}</div>
                  <article className="linkage-assistant-message">
                    <div className="linkage-speaker"><AgentIcon agent={configuration.controller.agentId}/><strong>{controller.agent}</strong><span>{linked ? '主控' : '对话'} · {controller.model}</span></div>
                    <p>{assistantText}</p>
                    {linked ? <button type="button" className={`linkage-task-summary linkage-task-${stage}`} onClick={() => { setPanelOpen(true); setTab('activity'); }}>
                      <span className="linkage-task-icon"><AgentIcon agent={configuration.executor.agentId}/></span>
                      <span><strong>{executor.agent}<small>执行 Agent</small></strong><span>{progressText}</span></span>
                      <span className="linkage-task-state">{stage === 'running' ? <LoaderCircle size={13} className="linkage-spin"/> : stage === 'completed' ? <Check size={14}/> : stage === 'waiting' ? <CircleHelp size={14}/> : stage === 'failed' ? <CircleAlert size={14}/> : <Square size={12}/>}<span>{stageNames[stage]}</span><ChevronRight size={13}/></span>
                    </button> : null}
                    {stage === 'waiting' ? <div className="linkage-question"><strong>保存后留在哪里？</strong><span>你的选择会交给执行 Agent 继续处理。</span><div><button type="button" onClick={() => setStage('running')}>留在设置页<Check size={13}/></button><button type="button" onClick={() => setStage('running')}>返回聊天<ChevronRight size={13}/></button></div></div> : null}
                    {stage === 'failed' ? <button type="button" className="quiet-button" onClick={() => setStage('running')}>预览继续执行<ArrowUp size={13}/></button> : null}
                    {stage === 'completed' ? <div className="linkage-result-note"><CheckCheck size={14}/><span>示例交付 · 改动与验证可在右侧展开查看</span></div> : null}
                  </article>
                </div>
              </>}
            </div>
            <div className="composer-area">
              <form className="composer" onSubmit={submit}>
                <div className="composer-input"><textarea aria-label="输入消息" rows={2} value={draft} placeholder={stage === 'setup' ? '随心输入，输入 / 查看命令与技能' : `继续和 ${controller.agent} 沟通…`}
                  onChange={event => setDraft(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!busy && catalogState === 'ready') previewConversation(); } }}/></div>
                <div className="composer-toolbar"><div className="composer-options">
                  <button type="button" className="icon-button" aria-label="填入示例需求" title="填入示例需求" onClick={() => setDraft(sampleRequest)}><Plus size={18}/></button>
                  <PermissionMenu value={permission} agent={linked ? configuration.executor.agentId : configuration.controller.agentId} busy={busy} onChange={setPermission}/>
                  <button type="button" className={`composer-chip linkage-entry${linked ? ' enabled' : ''}`} aria-label="联动设置" aria-haspopup="dialog" aria-expanded={showSettings} title={linked ? `${controller.agent} 沟通 · ${executor.agent} 执行` : '配置 Agent 联动'} onClick={() => setShowSettings(true)}><Link2 size={14}/><span>{linked ? '联动中' : '联动'}</span></button>
                </div><div className="composer-right">
                  <AgentMenu label="对话 Agent" value={configuration.controller.agentId} agents={catalogs.map(item => item.agent)} busy={busy} choose={id => {
                    const next = catalogs.find(item => item.agent.id === id); if (next) setConfiguration({ ...configuration, controller: firstLinkageChoice(next) });
                  }}/>
                  <LinkageModelPicker value={configuration.controller} catalog={controllerCatalog} disabled={busy} manage={() => setCatalogState('ready')}
                    onChange={controller => setConfiguration({ ...configuration, controller })}/>
                  {busy ? <button type="button" className="send-button" aria-label="停止整个联动" title="停止整个联动" onClick={event => {
                    // This DOM button becomes a submit button after the state
                    // update. Cancel click activation before changing its type.
                    event.preventDefault(); setStage('stopped');
                  }}><Square size={13}/></button>
                    : <button type="submit" className="send-button" aria-label="预览发送" disabled={catalogState !== 'ready'}><ArrowUp size={17}/></button>}
                </div></div>
              </form>
              <div className="composer-footnote"><span/><span>Enter 发送 · Shift Enter 换行</span></div>
            </div>
          </main>
        </div>
        {panel ? <aside className="linkage-execution-panel" aria-label="执行详情">
          <header><div><Terminal size={15}/><strong>执行详情</strong></div><button type="button" className="icon-button" aria-label="关闭执行详情" onClick={() => setPanelOpen(false)}><X size={16}/></button></header>
          <div className="linkage-detail-tabs" role="group" aria-label="执行详情视图"><button type="button" aria-pressed={tab === 'activity'} onClick={() => setTab('activity')}>执行过程</button><button type="button" aria-pressed={tab === 'changes'} onClick={() => setTab('changes')}>代码改动<span>2</span></button></div>
          <div className="linkage-detail-scroll">
            <div className="linkage-executor-identity"><AgentIcon agent={configuration.executor.agentId}/><div><strong>{executor.agent}</strong><span>{executor.model}</span></div><span className={`linkage-detail-status linkage-status-${stage}`}>{stageNames[stage]}</span></div>
            <div className="linkage-delegation"><span>来自 {controller.agent} 的任务</span><p>{request}</p></div>
            {tab === 'activity' ? <>
              <div className="linkage-timeline">{[
                { icon: Folder, title: '查看设置页与现有样式', detail: 'SettingsPage.tsx · settings.css' },
                { icon: FileCode2, title: '调整保存按钮的位置', detail: '保留原有按钮样式与保存行为' },
                { icon: Terminal, title: '检查类型与布局', detail: stage === 'completed' ? '示例：类型检查通过，布局已检查' : '完成修改后执行验证' },
                { icon: MessageSquare, title: '把执行结果交给主控', detail: '由主控整理后回复你' },
              ].map((item, index) => <div key={item.title} className={`linkage-timeline-item${index < step || stage === 'completed' ? ' done' : index === step && stage === 'running' ? ' current' : ''}`}>
                <span className="linkage-timeline-mark">{index < step || stage === 'completed' ? <Check size={12}/> : index === step && stage === 'running' ? <LoaderCircle size={12} className="linkage-spin"/> : <item.icon size={12}/>}</span><div><strong>{item.title}</strong><span>{item.detail}</span></div>
              </div>)}</div>
              {stage === 'failed' ? <div className="linkage-execution-error"><CircleAlert size={14}/><span>示例错误：模型连接中断，尚未完成验证。</span></div> : null}
              <button type="button" className="linkage-original-toggle" aria-expanded={showOriginal} onClick={() => setShowOriginal(v => !v)}><ChevronRight size={13} className={showOriginal ? 'is-open' : ''}/><span>查看执行 Agent 原始记录</span></button>
              {showOriginal ? <pre className="linkage-original">{'[示例执行记录]\nRead  src/SettingsPage.tsx\nRead  src/settings.css\nEdit  将保存操作移入页头\n\n这里只展示执行详情。\n主聊天中的表达由主控 Agent 负责。'}</pre> : null}
            </> : <div className="linkage-changes"><p>示例代码改动</p>{['src/SettingsPage.tsx', 'src/settings.css'].map(file => <div key={file}><FileCode2 size={14}/><span>{file}</span><small>修改</small></div>)}<pre><span>{'+ <div className="settings-page-actions">'}</span>{'\n+   <button>保存</button>\n+ </div>'}</pre></div>}
          </div>
          <footer><GitBranch size={13}/><span>与主控共用项目，分别保留会话</span><ArrowDown size={12}/></footer>
        </aside> : null}
      </div>
    </div>
    {showSettings ? <LinkageDialog value={{ ...configuration, mode: 'linked' }} catalogs={displayCatalogs} disabled={busy} onClose={() => setShowSettings(false)} manage={() => setCatalogState('ready')}
      onApply={value => { setConfiguration(value); setShowSettings(false); }}/>: null}
    {notice ? <div className="linkage-preview-notice" role="status"><span>{notice}</span><button type="button" aria-label="关闭提示" onClick={() => setNotice('')}><X size={14}/></button></div> : null}
  </div>;
}
