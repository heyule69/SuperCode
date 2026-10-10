import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Sparkles, ArrowUp, ArrowUpRight, FileCode2, Folder, GitBranch, LoaderCircle, MessageSquare, MoreHorizontal, PanelLeftOpen, PanelRightClose, PanelRightOpen, RefreshCw, Search, Square, SquarePen, Terminal, X } from 'lucide-react';
import { call, desktop, subscribe } from './api';
import { isDisplayEvent, mergeEvent } from './events';
import { Conversation, JumpToLatest } from './Conversation';
import { isActive, type Agent, type Bootstrap, type Changes, type Message, type Model, type ModelCatalog, type ModelSource, type PermissionMode, type Project, type RpcEvent, type RuntimeInfo, type Session, type SidebarState } from './types';
import { Sidebar } from './Sidebar';
import { emptySidebar } from './sidebarState';
import { commandMatches, parseCommand, slashCommands } from './slashCommands';
import { AddMenu, AttachmentStrip, ComposerSkills, ModelMenu, PermissionMenu, type Attachment } from './ComposerMenus';
import { UsageIndicator } from './UsageIndicator';
import { PlatformUsageIndicator } from './PlatformUsageView';
import type { TokenUsage, UsageRecord } from './usage';
import { applyPreferences, loadPreferences } from './preferences';
import { notificationPreferences } from './notifications';
import { compatiblePermission, loadPermissionChoices } from './permissions';
import { listen } from '@tauri-apps/api/event';
import { watchWindowTheme } from './windowTheme';
import { commandIndex, decodeDrafts, encodeDrafts, restoreWorkspace, type DraftModel } from './workspaceState';
import { CopyButton } from './CopyButton';
import { diffFiles } from './diff';
import { revealScrollItem, scrollToLatest } from './chatScroll';
import { systemDocument } from './links';
import type { ClientTab, Skill } from './ClientSettings';
import { version as appVersion } from '../package.json';
import AppUpdateSettings, { AppUpdateNotice } from './AppUpdate';
import type { ModelOption } from './modelPicker';
import { loadLocalSkills, skillSource, type SkillCatalog } from './skills';
import { DesktopTitleBar, type DesktopMenu } from './DesktopTitleBar';
import { emptyNavigation, travel, visit } from './navigation';
import type { SettingsTab } from './SettingsPage';
import { createChatSession } from './chatSession';
import { isProviderRequired, requireChatConnection } from './chatConnection';
import { FollowupQueue, useFollowups, type Followup, type FollowupPayload } from './Followups';
import { SelectionActions } from './SelectionActions';
import type { SideDraft } from './SideChat';
const SideChat=lazy(()=>import('./SideChat'));
import { ActivityRail } from './ActivityRail';
import { AgentMenu } from './AgentMenu';
import { NewChat } from './NewChat';
import { usePanelResize } from './PanelResize';
import { useChatReadReceipt } from './useChatReadReceipt';
import { clipboardImages, imageBase64, imageBytes, MAX_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES, mergeAttachments, readClipboardImages, validateImageFile } from './attachments';
const SettingsPage = lazy(() => import('./SettingsPage').then(module => ({ default: module.SettingsPage })));
const GeneralSettings = lazy(() => import('./SettingsPage').then(module => ({ default: module.GeneralSettings })));
const AgentSettings = lazy(() => import('./AgentSettings'));
const ClientSettings = lazy(() => import('./ClientSettings'));
const UsagePanel = lazy(() => import('./UsagePanel'));
const QuotaPanel = lazy(() => import('./QuotaPanel'));
const DiffView = lazy(() => import('./DiffView'));
const SourceView = lazy(() => import('./SourceView'));

const RequestCard = lazy(() => import('./RequestCard'));
const QuestionDock = lazy(() => import('./QuestionDock'));
const ProviderSettings = lazy(() => import('./ProviderSettings'));
const emptyChanges: Changes = { isGit: false, branch: '', files: [], diff: '' };
const statusText: Record<string, string> = { idle: '已就绪', starting: '正在启动', running: '正在工作', waiting: '等待确认', failed: '运行失败', interrupted: '已停止' };
function lastMessage(items: Message[], match: (m: Message) => boolean) { for (let i = items.length - 1; i >= 0; i--) if (match(items[i])) return items[i]; }

export default function App() {
  const [prefs, setPrefs] = useState(loadPreferences);
  const [ready, setReady] = useState(false);
  const [sideDraft,setSideDraft]=useState<SideDraft|null>(null);
  const [selectionDetail,setSelectionDetail]=useState('');
  const [detached] = useState(() => { const query = new URLSearchParams(location.search); return query.has('session') || query.get('window') === 'new'; });
  const [savedWorkspace] = useState(() => { const query = new URLSearchParams(location.search); const fresh = query.get('window') === 'new'; return { sessionId: fresh ? '' : query.get('session') ?? localStorage.getItem('supercode.session') ?? '', projectId: localStorage.getItem('supercode.project') ?? '', fresh }; });
  const [savedDrafts] = useState(() => decodeDrafts(localStorage.getItem('supercode.drafts.v1')));
  const prefsRef = useRef(prefs); prefsRef.current = prefs;
  const notificationOpenRef = useRef<(id?: string) => Promise<void>>(async () => {});
  const desktopNavigateRef = useRef<(request: { sessionId?: string; newChat?: boolean; search?: boolean }) => Promise<void>>(async () => {});
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const attachmentsRef = useRef(attachments); attachmentsRef.current = attachments;
  const attachmentDrafts = useRef(new Map([...savedDrafts].map(([key, draft]) => [key, draft.attachments])));
  const [importingImages, setImportingImages] = useState(false);
  const attachmentJobs = useRef(Promise.resolve());
  const pendingAttachments = useRef(0);
  const [effort, setEffort] = useState('');
  const [usage, setUsage] = useState<TokenUsage>();
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [sidebarState, setSidebarState] = useState<SidebarState>(emptySidebar);
  const [sidebarDialog, setSidebarDialog] = useState(false);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [profiles, setProfiles] = useState<Bootstrap['profiles']>([]);
  const [officialAgents, setOfficialAgents] = useState<string[]>([]);
  const [connectionOrder, setConnectionOrder] = useState<NonNullable<Bootstrap['connectionOrder']>>({});
  const [agentId, setAgentId] = useState(() => localStorage.getItem('supercode.agent') ?? prefs.defaultAgent);
  const [codexPath, setCodexPath] = useState('');
  const [configuring, setConfiguring] = useState(false);
  const [loadMcp, setLoadMcp] = useState(false);
  const [projectId, setProjectId] = useState('');
  const [sessionId, setSessionId] = useState('');
  const [messages, setMessages] = useState<Message[]>([]);
  const [requests, setRequests] = useState<RpcEvent[]>([]);
  const [input, setInput] = useState('');
  const [commandSelection, setCommandSelection] = useState(0);
  const [commandsDismissed, setCommandsDismissed] = useState(false);
  const [composerFocused, setComposerFocused] = useState(false);
  const [model, setModel] = useState('');
  const [models, setModels] = useState<Model[]>([]);
  const [modelSource, setModelSource] = useState<ModelSource>();
  const [modelSourceRevision, setModelSourceRevision] = useState('');
  const [connectionRevision, setConnectionRevision] = useState(0);
  const modelRequest = useRef(0);
  const [permissionChoices, setPermissionChoices] = useState(() => loadPermissionChoices(agentId, agentId === prefs.defaultAgent ? prefs.defaultPermission : 'ask'));
  const permissionMode = permissionChoices[agentId] ?? compatiblePermission(agentId === prefs.defaultAgent ? prefs.defaultPermission : 'ask', agentId);
  function setPermissionMode(mode: PermissionMode, targetAgent = agentId) { setPermissionChoices(current => ({ ...current, [targetAgent]: compatiblePermission(mode, targetAgent) })); }
  const readOnly = permissionMode === 'read';
  useEffect(() => { localStorage.setItem('supercode.permissions.v2', JSON.stringify(permissionChoices)); localStorage.setItem('supercode.permission', permissionMode); }, [permissionChoices, permissionMode]);
  const [commandMessage, setCommandMessage] = useState('');
  const [nativeCommands, setNativeCommands] = useState<string[]>([]);
  const [skillCatalog, setSkillCatalog] = useState<SkillCatalog>({ skills: [] });
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState('');
  const [loadingModels, setLoadingModels] = useState(false);
  const [sending, setSending] = useState(false);
  const [sidebar, setSidebar] = useState(() => localStorage.getItem('supercode.sidebar') !== 'false');
  const [context, setContext] = useState(() => localStorage.getItem('supercode.context') === 'true');
  const [narrow, setNarrow] = useState(() => window.matchMedia('(max-width: 1100px)').matches);
  const [contextOverlay, setContextOverlay] = useState(false);
  const showContext = narrow ? contextOverlay : context;
  const [autoExpand, setAutoExpand] = useState(() => localStorage.getItem('supercode.expandActivity.compact') === 'true');
  const [settingsTab, setSettingsTab] = useState<SettingsTab>('general');
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsOpenRef = useRef(false); settingsOpenRef.current = settingsOpen;
  const settingsScroll = useRef(0);
  const [navigation, setNavigation] = useState(emptyNavigation);
  const navigating = useRef(false);
  const navigationAction = useRef<(direction: -1 | 1) => void>(() => {});
  const editTarget = useRef<HTMLElement | null>(null);
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(() => { try { return new Set(JSON.parse(localStorage.getItem('supercode.collapsedProjects') ?? '[]')); } catch { return new Set(); } });
  const [showJump, setShowJump] = useState(false);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [changes, setChanges] = useState<Changes>(emptyChanges);
  const [changesLoading, setChangesLoading] = useState(false);
  const [preview, setPreview] = useState<{ name: string; text: string; diff?: boolean; line?: number } | null>(null);
  const [error, setError] = useState('');
  const [modal, setModal] = useState<'search' | 'project' | 'rename' | 'provider' | null>(null);
  const [providerRequiredAgent, setProviderRequiredAgent] = useState('');
  const [providerSettingsAgent, setProviderSettingsAgent] = useState<string>();
  const modalRef = useRef(modal); modalRef.current = modal;
  const [modalError, setModalError] = useState('');
  const [modalWorking, setModalWorking] = useState(false);
  const [pathInput, setPathInput] = useState('');
  const [search, setSearch] = useState('');
  const [rename, setRename] = useState('');
  const [hasMore, setHasMore] = useState(false);
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const sessionsRef = useRef(sessions); sessionsRef.current=sessions;
  const current = useRef({ sessionId: '', projectId: '', nativeId: null as string | null });
  const pendingSession = useRef('');
  const queue = useRef<RpcEvent[]>([]);
  const flushTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scrollArea = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const prependScroll = useRef<{ height: number; top: number } | null>(null);
  const drafts = useRef(new Map([...savedDrafts].map(([key, draft]) => [key, draft.text])));
  const draftKey = useRef('');
  const inputRef = useRef(input); inputRef.current = input;
  const changesRevision = useRef(0);
  const modalElement = useRef<HTMLElement>(null);
  const newSessionRef = useRef(() => {});
  const draftModels = useRef(new Map<string, DraftModel>([...savedDrafts].flatMap(([key, draft]) => draft.selection ? [[key, draft.selection] as [string, DraftModel]] : [])));
  const [draftConnectionId, setDraftConnectionId] = useState<string>();
  const sidebarUpdateRef = useRef(() => Promise.resolve());
  const textarea = useRef<HTMLTextAreaElement>(null);
  const project = projects.find(p => p.id === projectId);
  const session = sessions.find(s => s.id === sessionId);
  const workspacePath = project?.path ?? session?.workspacePath ?? undefined;
  const panels = usePanelResize(sidebar, !!(showContext || sideDraft), settingsOpen);
  useChatReadReceipt({ sessionId, status: session?.status, unread: !!sidebarState.sessions[sessionId]?.unread, throughSeq: messages.reduce((seq, message) => Math.max(seq, message.seq), 0), loading: messagesLoading, settingsOpen,
    onRead: id => setSidebarState(old => ({ ...old, sessions: { ...old.sessions, [id]: { ...old.sessions[id], unread: false } } })), onError: report });
  const busy = (sending && pendingSession.current === sessionId) || isActive(session?.status);
  const anyBusy = sending || sessions.some(s => isActive(s.status));
  const connectionId = session?.connectionId ?? draftConnectionId ?? profiles.find(p => p.agent === agentId && p.current)?.id ?? (officialAgents.includes(agentId) ? '@official' : '@local');
  const activeProfile = profiles.find(p => p.agent === agentId && p.id === connectionId);
  const [handoffTarget, setHandoffTarget] = useState('');
  const profileRevision = `${agentId}:${sessionId}:${connectionId}:${JSON.stringify(activeProfile ?? null)}:${officialAgents.join(',')}:${connectionRevision}`;
  const currentModelSource = modelSourceRevision === profileRevision ? modelSource : undefined;
  const skillQuery = input.startsWith('/');
  const slashOptions = commandsDismissed || !composerFocused ? [] : commandMatches(input, agentId, nativeCommands, skillCatalog.skills);
  const slashIndex = Math.min(commandSelection, Math.max(0, slashOptions.length - 1));
  useEffect(() => {
    const capabilities = messages.find(m => m.kind === 'agentCapabilities');
    if (capabilities) setNativeCommands(capabilities.data?.commands as string[] ?? []);
  }, [messages]);
  useEffect(() => { setNativeCommands([]); }, [sessionId]);
  useEffect(() => { setSkillCatalog({ skills: [] }); setSkillsError(''); }, [projectId]);
  useEffect(() => {
    const changed = (event: Event) => { const detail = (event as CustomEvent<{ projectId: string; catalog: SkillCatalog }>).detail; if (detail.projectId === projectId) setSkillCatalog(detail.catalog); };
    window.addEventListener('supercode:skills-updated', changed);
    return () => window.removeEventListener('supercode:skills-updated', changed);
  }, [projectId]);
  useEffect(() => {
    if (!ready || !desktop || !skillQuery) { setSkillsLoading(false); return; }
    let disposed = false; setSkillsLoading(true); setSkillsError('');
    void loadLocalSkills(projectId).then(catalog => { if (!disposed) setSkillCatalog(catalog); }).catch(e => { if (!disposed) setSkillsError(String(e)); }).finally(() => { if (!disposed) setSkillsLoading(false); });
    return () => { disposed = true; };
  }, [ready, projectId, skillQuery]);
  useEffect(() => {
    let disposed = false;
    const request = ++modelRequest.current;
    setModels([]); setModelSource(undefined); setModelSourceRevision(profileRevision);
    if (!ready || !desktop) { setLoadingModels(false); return; }
    setLoadingModels(true);
    const args = { agent: agentId, sessionId: sessionId || null, connectionId };
    void (async () => {
      const source = await call<ModelSource>('get_model_source', args);
      if (disposed || request !== modelRequest.current) return;
      setModelSource(source);
      if (source.available !== true) return;
      // API catalogs are stored locally. Official Codex discovery stays on demand.
      const automatic = agentId === 'claude' || !!activeProfile && !activeProfile.officialAccount || ['opencode', 'pi'].includes(agentId);
      if (!automatic) return;
      const result = await call<ModelCatalog>('list_models', args);
      if (disposed || request !== modelRequest.current) return;
      setModels(result.data); setModelSource(result.source);
      setModel(previous => session?.model ? session.model : result.data.some(m => m.model === previous) ? previous : result.data.find(m => m.isDefault)?.model ?? '');
    })().catch(e => { if (!disposed && request === modelRequest.current) report(e); }).finally(() => { if (!disposed && request === modelRequest.current) setLoadingModels(false); });
    return () => { disposed = true; };
  }, [profileRevision, ready]);
  const currentRequests = requests.filter(r => !r.params.threadId || r.params.threadId === session?.nativeId || r.params.threadId === current.current.nativeId);
  const questionPending = currentRequests.some(request => request.method === 'item/tool/requestUserInput');
  async function respondRequest(request: RpcEvent, result: unknown) {
    try { await call('respond_request', { id: request.id, result }); setRequests(old => old.filter(r => r.id !== request.id)); }
    catch (e) { report(e); throw e; }
  }
  const newChat = ready && !messages.length && !busy && !messagesLoading && !currentRequests.length;
  const activeElsewhere = sessions.find(s => isActive(s.status) && s.id !== sessionId);

  const latest = lastMessage(messages, m => m.kind !== 'runMarker' && m.role !== 'user' && (!session?.turnId || m.data?.turnId === session.turnId));
  const agentName = agents.find(a => a.id === agentId)?.name ?? agentId;
  newSessionRef.current = () => { void newSession(); };

  function showProviderRequired(agent = agentId) {
    if (agent === agentId) { modelRequest.current++; setModels([]); setModelSource({ providerId: 'unknown', providerName: '', mark: '', available: false }); setModelSourceRevision(profileRevision); setLoadingModels(false); }
    setProviderRequiredAgent(agent); setError(''); setModal('provider');
  }
  function report(e: unknown) { if (isProviderRequired(e)) { showProviderRequired(); return; } const text = e instanceof Error ? e.message : String(e); if (modalRef.current || settingsOpenRef.current) setModalError(text); else setError(text); }
  const showDiff = useCallback((name: string, text: string) => { setPreview({ name, text, diff: true }); if (narrow) setContextOverlay(true); else setContext(true); }, [narrow]);
  const refresh = useCallback(async () => {
    const data = await call<Bootstrap>('bootstrap');
    setProjects(data.projects); setSessions(data.sessions); setAgents(data.agents);
    setSidebarState(data.sidebar ?? emptySidebar);
    setProfiles(data.profiles ?? []);
    setOfficialAgents(data.officialAgents ?? []);
    setConnectionOrder(data.connectionOrder ?? {});
    setCodexPath(data.codexPath ?? '');
    setLoadMcp(data.loadMcp ?? false);
    return data;
  }, []);
  const loadMessages = useCallback(async (id: string, before?: number) => {
    setMessagesLoading(true);
    try {
    const items = await call<Message[]>('list_messages', { sessionId: id, before: before ?? null });
    if (current.current.sessionId !== id) return;
    if (before && scrollArea.current) prependScroll.current = { height: scrollArea.current.scrollHeight, top: scrollArea.current.scrollTop };
    setMessages(old => before ? [...items.filter(m => !old.some(v => v.id === m.id)), ...old] : [...items.map(saved => {
      const live = old.find(m => m.id === saved.id);
      return live && live.text.length > saved.text.length && ['inProgress', 'preparing'].includes(String(live.data?.status)) && ['inProgress', 'preparing'].includes(String(saved.data?.status)) ? { ...live, seq: saved.seq } : saved;
    }), ...old.filter(m => m.seq === 0 && m.role !== 'user' && !items.some(saved => saved.id === m.id))]); setHasMore(items.length === 100);
    } finally { if (current.current.sessionId === id) setMessagesLoading(false); }
  }, []);
  const refreshUsage = useCallback(async (id: string) => {
    const result=await call<{records:UsageRecord[]}>('get_usage',{sessionId:id});
    if(current.current.sessionId===id)setUsage(result.records[0]?.data);
  }, []);
  const refreshChanges = useCallback(async (id: string) => {
    if (!id && !current.current.sessionId) return;
    const selectedSession = current.current.sessionId;
    const revision = ++changesRevision.current;
    setChangesLoading(true);
    try { const data = await call<Changes>('workspace_changes', { projectId: id, sessionId: selectedSession || null }); if (current.current.projectId === id && current.current.sessionId === selectedSession && revision === changesRevision.current) setChanges(data); }
    catch (e) { report(e); }
    finally { if (revision === changesRevision.current) setChangesLoading(false); }
  }, []);
  const flush = useCallback(() => {
    if (flushTimer.current) clearTimeout(flushTimer.current);
    flushTimer.current = null;
    const events = queue.current.splice(0);
    const activeId = current.current.sessionId;
    if (events.length) setMessages(old => events.reduce((items, event) => mergeEvent(items, event, activeId), old));
  }, []);

  useEffect(() => {
    let disposed = false; let off: (() => void) | undefined;
    void refresh().then(data => {
      if (disposed) return;
      const { session: first, project: p } = restoreWorkspace(data, savedWorkspace);
      if (p) { setProjectId(p.id); current.current.projectId = p.id; }
      if (first) { setSessionId(first.id); setAgentId(first.agent); setModel(first.model ?? ''); draftKey.current = first.id; current.current = { sessionId: first.id, projectId: first.projectId, nativeId: first.nativeId }; void loadMessages(first.id).catch(report); }
      else {
        draftKey.current = `${p?.id ?? ''}:${localStorage.getItem('supercode.agent') ?? 'claude'}`;
        const selection = draftModels.current.get(draftKey.current);
        setDraftConnectionId(selection?.connectionId); setModel(selection?.model ?? ''); setEffort(selection?.effort ?? '');
      }
      setInput(drafts.current.get(draftKey.current) ?? ''); setAttachments(attachmentDrafts.current.get(draftKey.current) ?? []); setReady(true);
      if (desktop) {
        void call<RpcEvent[]>('pending_requests').then(pending => { if (!disposed) setRequests(old => [...pending.filter(r => !old.some(p => p.id === r.id)),...old]); }).catch(report);
        void call<boolean>('get_ui_recovery').then(recovered => { if (!disposed && recovered) setCommandMessage('页面已恢复，会话和任务已保留。'); }).catch(report);
      }
    }).catch(report);
    void subscribe(event => {
      if (disposed) return;
      const p = event.params;
      const native = p.threadId ?? p.thread?.id;
      if (event.method === 'agent/commands' && native === current.current.nativeId) {
        setNativeCommands(Array.isArray(p.commands) ? p.commands as string[] : []);
        return;
      }
      if (['thread/goal/updated', 'thread/goal/cleared'].includes(event.method) && native === current.current.nativeId) {
        const goal = p.goal as { objective?: string; status?: string; tokensUsed?: number } | undefined;
        const names: Record<string, string> = { active: '执行中', paused: '已暂停', complete: '已完成', blocked: '等待解决阻碍', budgetLimited: '已达到目标预算', usageLimited: '已达到用量限制' };
        setCommandMessage(goal ? `目标：${goal.objective}\n状态：${names[goal.status ?? ''] ?? goal.status}\n已用 Token：${goal.tokensUsed ?? 0}` : '目标已清除。');
        return;
      }
      if (pendingSession.current && native && ['thread/started', 'turn/started'].includes(event.method)) {
        if (current.current.sessionId === pendingSession.current) current.current.nativeId = native;
        setSessions(old => old.map(s => s.id === pendingSession.current ? { ...s, nativeId: native } : s));
      }
      const matches = native === current.current.nativeId;
      if (matches && event.method === 'thread/tokenUsage/updated') setUsage(p.tokenUsage);
      if (event.id !== undefined) { setRequests(old => [...old.filter(r => r.id !== event.id), event]); setSessions(old => old.map(s => s.nativeId === native ? { ...s, status: 'waiting' } : s)); }
      if (event.method === 'serverRequest/resolved') { setRequests(old => old.filter(r => r.id !== p.requestId)); setSessions(old => old.map(s => s.nativeId === native && s.status === 'waiting' ? { ...s, status: 'running' } : s)); }
      if (event.method === 'turn/started') { setSessions(old => old.map(s => s.nativeId === native ? { ...s, status: 'running', turnId: p.turn.id } : s)); if(matches)void loadMessages(current.current.sessionId).catch(report); }
      if (matches && isDisplayEvent(event)) {
        queue.current.push(event); if (!flushTimer.current) flushTimer.current = setTimeout(flush, 50);
      }
      if (event.method === 'error' && matches) setError(p.error?.message ?? 'Agent 运行失败');
      if (event.method === 'turn/completed') {
        if (matches) { flush(); void loadMessages(current.current.sessionId).catch(report); void refreshUsage(current.current.sessionId).catch(report); void refreshChanges(current.current.projectId); }
        setRequests(old => old.filter(r => r.params.turnId !== p.turn.id));
        void refresh().catch(report);
      }
      if (event.method === 'turn/diff/updated' && matches) setChanges(old => ({ ...old, diff: p.diff ?? old.diff }));
    }, text => setLogs(old => [...old.slice(-199), text]), () => {
      setRequests([]); setRuntime(old => old ? { ...old, running: false } : old); flush(); void refresh().then(() => current.current.sessionId ? loadMessages(current.current.sessionId) : undefined).catch(report);
    }, () => { void sidebarUpdateRef.current().catch(report); }).then(cleanup => { if (disposed) cleanup(); else off = cleanup; }).catch(report);
    return () => { disposed = true; off?.(); if (flushTimer.current) clearTimeout(flushTimer.current); queue.current = []; };
  }, [refresh, loadMessages, refreshChanges, refreshUsage, flush]);

  useEffect(() => { if ((projectId || sessionId) && context) void refreshChanges(projectId); }, [projectId, sessionId, context, refreshChanges]);
  useLayoutEffect(() => { applyPreferences(prefs); const query = window.matchMedia('(prefers-color-scheme: dark)'); const update = () => applyPreferences(prefs); query.addEventListener('change', update); return () => query.removeEventListener('change', update); }, [prefs]);
  useEffect(() => {
    if (desktop) void call('update_notification_preferences', { preferences: notificationPreferences(prefs) }).catch(report);
  }, [prefs]);
  useEffect(() => {
    const changed = (event: StorageEvent) => { if (event.key === 'supercode.preferences') { const next = loadPreferences(); if (JSON.stringify(next) !== JSON.stringify(prefsRef.current)) setPrefs(next); } };
    window.addEventListener('storage', changed); return () => window.removeEventListener('storage', changed);
  }, []);
  useEffect(() => {
    if (!desktop) return;
    const update = () => { void call('update_notification_context', { context: { sessionId: current.current.sessionId, settingsOpen: settingsOpenRef.current } }).catch(report); };
    update(); window.addEventListener('focus', update); document.addEventListener('visibilitychange', update);
    return () => { window.removeEventListener('focus', update); document.removeEventListener('visibilitychange', update); };
  }, [ready, sessionId, settingsOpen]);
  useEffect(() => {
    if (!desktop || detached) return;
    let disposed = false; let off: (() => void) | undefined;
    void listen<{ sessionId?: string }>('notification-open', event => { if (!disposed) void notificationOpenRef.current(event.payload.sessionId).catch(report); }).then(cleanup => { if (disposed) cleanup(); else off = cleanup; }).catch(report);
    return () => { disposed = true; off?.(); };
  }, [detached]);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; let off: (() => void) | undefined;
    void listen<{ sessionId?: string; newChat?: boolean; search?: boolean }>('desktop-navigate', event => { if (!disposed) void desktopNavigateRef.current(event.payload).catch(report); }).then(cleanup => { if (disposed) cleanup(); else off = cleanup; }).catch(report);
    return () => { disposed = true; off?.(); };
  }, []);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false; let off: (() => void) | undefined;
    void listen('desktop-before-exit', () => { if (!disposed) window.dispatchEvent(new Event('beforeunload')); }).then(cleanup => { if (disposed) cleanup(); else off = cleanup; }).catch(report);
    return () => { disposed = true; off?.(); };
  }, []);
  useEffect(() => {
    if (!desktop) return;
    return watchWindowTheme(prefs.theme, report);
  }, [prefs.theme]);
  useEffect(() => {
    if (settingsOpen && settingsTab === 'resources' && desktop) void inspectRuntime();
  }, [settingsOpen, settingsTab]);
  useEffect(() => { setUsage(undefined); if (!desktop || !sessionId) return; let disposed=false; void call<{records:UsageRecord[]}>('get_usage',{sessionId}).then(r=>{if(!disposed)setUsage(r.records[0]?.data);}).catch(report); return ()=>{disposed=true;}; }, [sessionId]);
  useEffect(() => { const query = window.matchMedia('(max-width: 1100px)'); const changed = () => { setNarrow(query.matches); setContextOverlay(false); }; query.addEventListener('change', changed); return () => query.removeEventListener('change', changed); }, []);
  useEffect(() => { localStorage.setItem('supercode.agent', agentId); }, [agentId]);
  useEffect(() => { if (ready && !detached) { localStorage.setItem('supercode.session', sessionId); localStorage.setItem('supercode.project', projectId); } }, [ready, sessionId, projectId, detached]);
  useEffect(() => {
    if (!ready || !draftKey.current) return;
    const key = draftKey.current;
    if (!sessionId && draftConnectionId) draftModels.current.set(key, { connectionId: draftConnectionId, model, effort });
    while (draftModels.current.size > 20) draftModels.current.delete(draftModels.current.keys().next().value!);
    const save = () => { if (draftKey.current !== key) return; drafts.current.delete(key); attachmentDrafts.current.delete(key); if (input || attachments.length) { drafts.current.set(key, input.slice(0, 32000)); attachmentDrafts.current.set(key, attachments); } while (drafts.current.size > 20) { const oldest = drafts.current.keys().next().value!; drafts.current.delete(oldest); attachmentDrafts.current.delete(oldest); } try { localStorage.setItem('supercode.drafts.v1', encodeDrafts(drafts.current, attachmentDrafts.current, draftModels.current)); } catch { /* Current in-memory drafts remain available if storage is full. */ } };
    const timer = setTimeout(save, 250);
    const leaving = () => save();
    window.addEventListener('beforeunload', leaving);
    return () => { clearTimeout(timer); window.removeEventListener('beforeunload', leaving); if (inputRef.current === input && attachmentsRef.current === attachments) save(); };
  }, [ready, input, attachments, sessionId, projectId, agentId, draftConnectionId, model, effort]);
  useEffect(() => { setCommandSelection(0); if (!input.startsWith('/')) setCommandsDismissed(false); }, [input]);
  useEffect(() => { setCommandsDismissed(false); }, [agentId]);
  useEffect(() => { revealScrollItem(document.getElementById('agent-commands'), document.getElementById(`agent-command-${slashIndex}`)); }, [slashIndex, slashOptions.length]);
  useEffect(() => { setModalError(''); }, [modal, settingsOpen, settingsTab]);
  useEffect(() => {
    if (!ready || navigating.current) return;
    setNavigation(old => visit(old, settingsOpen ? { page: 'settings', tab: settingsTab } : { page: 'chat', projectId, sessionId }));
  }, [ready, settingsOpen, settingsTab, projectId, sessionId]);
  useEffect(() => {
    const remember = (event: FocusEvent) => { const target = event.target as HTMLElement; if (!target.closest('.desktop-titlebar')) editTarget.current = target; };
    document.addEventListener('focusin', remember);
    return () => document.removeEventListener('focusin', remember);
  }, []);
  useLayoutEffect(() => {
    if (settingsOpen) document.querySelector<HTMLInputElement>('.settings-search input')?.focus({ preventScroll: true });
    else if (scrollArea.current) { scrollArea.current.scrollTop = settingsScroll.current; }
  }, [settingsOpen]);
  useEffect(() => { localStorage.setItem('supercode.sidebar', String(sidebar)); localStorage.setItem('supercode.context', String(context)); localStorage.setItem('supercode.expandActivity.compact', String(autoExpand)); localStorage.setItem('supercode.collapsedProjects', JSON.stringify([...collapsedProjects])); }, [sidebar, context, autoExpand, collapsedProjects]);
  useLayoutEffect(() => {
    const el = scrollArea.current;
    if (prependScroll.current && el) { el.scrollTop = prependScroll.current.top + el.scrollHeight - prependScroll.current.height; prependScroll.current = null; }
    else if (stickToBottom.current) scrollToLatest(el);
  }, [messages, currentRequests.length]);
  useLayoutEffect(() => { const el = textarea.current; if (el) { el.style.height = 'auto'; el.style.height = `${questionPending && !input ? 28 : Math.min(196, Math.max(questionPending ? 28 : 48, el.scrollHeight))}px`; } }, [input, attachments, questionPending]);
  useLayoutEffect(() => {
    const area = scrollArea.current;
    const content = area?.querySelector('.messages');
    if (!area || !content) return;
    const observer = new ResizeObserver(() => {
      if (!settingsOpenRef.current && stickToBottom.current) { scrollToLatest(area); setShowJump(false); }
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [ready, messagesLoading, sessionId]);
  useEffect(() => {
    if (!modal) return;
    const previous = document.activeElement as HTMLElement | null;
    const el = modalElement.current;
    const initial = el?.querySelector<HTMLElement>('input:not([type=checkbox])') ?? el?.querySelector<HTMLElement>('button');
    initial?.focus();
    function trap(e: KeyboardEvent) {
      if (e.key !== 'Tab' || !el) return;
      const items = Array.from(el.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]')).filter(v => v.offsetParent !== null);
      const first = items[0], last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    }
    window.addEventListener('keydown', trap); return () => { window.removeEventListener('keydown', trap); if (previous?.isConnected) previous.focus(); else textarea.current?.focus(); };
  }, [modal]);
  useEffect(() => {
    function shortcuts(e: KeyboardEvent) {
      if (sidebarDialog) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); setModal('search'); }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n') { e.preventDefault(); newSessionRef.current(); }
      if ((e.ctrlKey || e.metaKey) && (e.key === ',' || e.code === 'Comma')) { e.preventDefault(); openSettings(); }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') { e.preventDefault(); setSidebar(v => !v); }
      if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight') && !modalRef.current) { e.preventDefault(); navigationAction.current(e.key === 'ArrowLeft' ? -1 : 1); }
      if (e.key === 'Escape' && !modalWorking) { if (modalRef.current) setModal(null); else if (settingsOpenRef.current) setSettingsOpen(false); else setPreview(null); }
    }
    window.addEventListener('keydown', shortcuts); return () => window.removeEventListener('keydown', shortcuts);
  }, [modalWorking, sidebarDialog]);

  async function addProject(path?: string, browse = false) {
    try {
      if (!path && desktop && browse) {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({ directory: true, multiple: false, title: '选择项目文件夹' });
        if (!selected) return; path = selected;
      }
      if (!path) { setModal('project'); return; }
      const p = await call<Project>('add_project', { path }); await refresh(); selectProject(p); setModal(null); setPathInput('');
    } catch (e) { report(e); }
  }
  function selectProject(p: Project | undefined, nextAgent = agentId) {
    setSettingsOpen(false);
    const sameProject = current.current.projectId === (p?.id ?? '');
    drafts.current.set(draftKey.current, inputRef.current);
    attachmentDrafts.current.set(draftKey.current, attachmentsRef.current);
    draftKey.current = `${(p?.id ?? '')}:${nextAgent}`;
    const selection = draftModels.current.get(draftKey.current);
    setAgentId(nextAgent); setModels([]); modelRequest.current++;
    setDraftConnectionId(selection?.connectionId); setModel(selection?.model ?? ''); setModelSource(undefined); setUsage(undefined); setEffort(selection?.effort ?? '');
    setAttachments(attachmentDrafts.current.get(draftKey.current) ?? []); setInput(drafts.current.get(draftKey.current) ?? '');
    flush(); setProjectId((p?.id ?? '')); setSessionId(''); setMessages([]); setPreview(null); setChanges(emptyChanges);
    setHasMore(false); setShowJump(false); setMessagesLoading(false); stickToBottom.current = true; changesRevision.current++;
    current.current = { sessionId: '', projectId: (p?.id ?? ''), nativeId: null }; setError('');
    if (sameProject && context) void refreshChanges((p?.id ?? ''));
  }
  async function selectSession(s: Session) {
    setSettingsOpen(false);
    drafts.current.set(draftKey.current, inputRef.current);
    attachmentDrafts.current.set(draftKey.current, attachmentsRef.current); setAttachments(attachmentDrafts.current.get(s.id) ?? []); setEffort('');
    setDraftConnectionId(undefined);
    draftKey.current = s.id; setInput(drafts.current.get(s.id) ?? ''); setHasMore(false); setShowJump(false);
    flush(); setSessionId(s.id); setProjectId(s.projectId); setMessages([]); setPreview(null); setAgentId(s.agent); setModels([]); setModelSource(profiles.find(p => p.id === s.connectionId)?.modelSource); setModel(s.model ?? ''); setUsage(undefined); setError(''); stickToBottom.current = true;
    current.current = { sessionId: s.id, projectId: s.projectId, nativeId: s.nativeId }; setModal(null);
    try { await loadMessages(s.id); } catch (e) { report(e); }
  }
  notificationOpenRef.current = async id => {
    if (!id) { setSettingsTab('notifications'); openSettings(); return; }
    const data = await refresh(); const target = data.sessions.find(s => s.id === id);
    if (target) { setSidebarDialog(false); await selectSession(target); }
    else setCommandMessage('这条通知对应的对话已归档或移除，可在已归档中查找。');
  };
  desktopNavigateRef.current = async request => {
    setSidebarDialog(false); setSettingsOpen(false);
    if (request.sessionId) await notificationOpenRef.current(request.sessionId);
    else if (request.search) setModal('search');
    else if (request.newChat) await newSession();
  };
  async function newSession(target?: Project) {
    if (anyBusy || configuring) return;
    // Repeated clicks keep the same unsent draft, including its model and attachments.
    if (sessionId || projectId !== (target?.id ?? '')) { selectProject(target, prefsRef.current.defaultAgent); setPermissionMode(compatiblePermission(prefsRef.current.defaultPermission, prefsRef.current.defaultAgent), prefsRef.current.defaultAgent); }
    if (target) setCollapsedProjects(old => { const next = new Set(old); next.delete(target.id); return next; });
    setModal(null); setError('');
    requestAnimationFrame(() => textarea.current?.focus());
  }
  const followups=useFollowups(sessionId,report);
  function followupPayload(text:string,taskAttachments=attachments):FollowupPayload { return {text,model:model||null,readOnly,permissionMode,attachments:taskAttachments,effort:effort||null}; }
  async function changeFollowup(id:string,action:string,text?:string){try{await call('change_followup',{id,action,text:text??null});await followups.reload();}catch(e){report(e);throw e;}}
  async function steerFollowup(id:string){try{await call('steer_followup',{id,expectedTurnId:session?.turnId});await followups.reload();await refresh();await loadMessages(sessionId);}catch(e){report(e);}}
  async function openSide(row?:Followup,text='') {
    if(sideDraft){setError('请先收起当前侧边聊天，再打开新的内容。');return;}
    try{if(row)await changeFollowup(row.id,'pause');setContext(false);setContextOverlay(false);setSideDraft({key:crypto.randomUUID(),text:row?.payload.text??text,payload:row?.payload,sourceId:row?.id,projectId,agent:agentId,connectionId});}catch(e){report(e);}
  }
  async function send() {
    if (pendingAttachments.current) return;
    if (!input.trim() && !attachments.length) return;
    const command = parseCommand(input);
    let availableSkills = skillCatalog.skills;
    if (command && desktop && !slashCommands.some(c => c.name === command.command && (!c.agent || c.agent === agentId))) {
      try { availableSkills = (await loadLocalSkills(projectId)).skills; } catch (e) { report(e); return; }
    }
    const commandSkill = command ? commandMatches('/', agentId, nativeCommands, availableSkills).find(c => c.name === command.command)?.skill : undefined;
    if (command && !commandSkill) { await handleCommand(command.command, command.argument); return; }
    if (sending) return;
    if (!desktop) { setError('当前是浏览器界面预览。运行 npm run desktop 后，可以使用真实的本地 Agent。'); return; }
    const taskAttachments = commandSkill && !attachments.some(a => a.path === commandSkill.path) ? [...attachments, { kind: 'skill' as const, name: commandSkill.name, path: commandSkill.path }] : attachments;
    if (taskAttachments.length > 12) { setError('最多添加 12 个附件，请先移除一个附件。'); return; }
    const originalInput = input;
    const text = commandSkill ? command!.argument || `请执行所选技能 ${commandSkill.name}。` : input.trim() || (attachments.some(a => a.kind === 'skill') ? '请执行所选技能。' : '请查看附件。'); let id = sessionId;
    const originDraft = draftKey.current;
    setSending(true);
    try {
      await requireChatConnection(agentId, connectionId, id);
      if (!id) { const s = await createChatSession({ projectId, model, agent: agentId, connectionId }); id = s.id; setSessions(old => [s, ...old]); if(current.current.projectId===projectId && draftKey.current===originDraft){setSessionId(id);current.current.sessionId=id;draftKey.current=id;} }
      if (anyBusy || followups.rows.length) {
        await call('enqueue_followup',{sessionId:id,payload:followupPayload(text,taskAttachments)});
        if(current.current.sessionId===id){setInput('');setAttachments([]);}
        drafts.current.delete(originDraft);drafts.current.delete(id);attachmentDrafts.current.delete(originDraft);attachmentDrafts.current.delete(id);
        await followups.reload();await refresh();return;
      }
      pendingSession.current = id; drafts.current.delete(originDraft); drafts.current.delete(id); draftModels.current.delete(originDraft); setError('');
      const optimisticId = `optimistic-${crypto.randomUUID()}`;
      if(current.current.sessionId===id){setInput('');stickToBottom.current=true;setMessages(old => [...old, { seq: 0, id: optimisticId, sessionId: id, role: 'user', text, kind: 'userMessage', data: {attachments:taskAttachments} }]);}
      const result = await call<Session>('send_chat_message', { sessionId: id, text, model: model || null, readOnly, permissionMode, attachments: taskAttachments, effort: effort || null });
      if (current.current.sessionId === id) setAttachments([]); attachmentDrafts.current.delete(originDraft); attachmentDrafts.current.delete(id);
      setSessions(old => old.map(s => s.id === id ? result : s));
      if (current.current.sessionId === id) { current.current.nativeId = result.nativeId; setModel(result.model ?? ''); }
      await loadMessages(id); await refresh();
    } catch (e) { report(e); if (current.current.sessionId === id) setInput(originalInput); else drafts.current.set(id || originDraft, originalInput); await refresh().catch(report); if (id) await loadMessages(id).catch(report); }
    finally { pendingSession.current = ''; setSending(false); }
  }
  async function handleCommand(command: string, argument: string) {
    setError(''); setCommandMessage('');
    if (command === '/stop') { if (busy) await stop(); else setCommandMessage('当前会话没有运行中的任务。'); setInput(''); return; }
    const goalControl = agentId === 'codex' && busy && (command === '/goal-status' || (command === '/goal' && ['', 'status', 'pause', 'resume', 'clear'].includes(argument)));
    if (anyBusy && !goalControl) { setError('请先停止或完成当前任务，再使用命令。'); return; }
    if (['/new', '/clear'].includes(command)) { setInput(''); await newSession(); return; }
    if (command === '/compact') {
      if (!sessionId || !session?.model) { setError('先发送一条消息，再压缩上下文'); return; }
      setConfiguring(true); setHandoffTarget('@compact');
      const id=sessionId;
      try {
        const saved=await call<Session>('compact_session_context',{sessionId:id});
        if (current.current.sessionId===id) { current.current.nativeId=saved.nativeId; setUsage(undefined); setInput(''); await loadMessages(id); }
        await refresh();
      } catch(e) { report(e); await refresh(); }
      finally { setConfiguring(false); setHandoffTarget(''); }
      return;
    }
    if (command === '/model') { setSettingsTab('providers'); openSettings(); setInput(''); return; }
    if (command === '/permissions') { document.querySelector<HTMLButtonElement>('[aria-label="权限模式"]')?.click(); setInput(''); return; }
    if (command === '/status') { await inspectRuntime(); setCommandMessage(`${agentId === 'claude' ? 'Claude Code' : 'Codex'} · ${statusText[session?.status ?? 'idle']} · ${activeProfile?.name ?? (officialAgents.includes(agentId) ? '官方账号' : '本机 CLI 配置')} · ${model || '默认模型'}`); setInput(''); return; }
    if (command === '/help') { setCommandMessage(commandMatches('/', agentId, nativeCommands, skillCatalog.skills).map(c => `${c.name} — ${c.skill ? '技能 · ' : ''}${c.detail}`).join('\n')); setInput(''); return; }
    if ((agentId === 'claude' || ['opencode', 'pi'].includes(agentId) && command !== '/compact' && !slashCommands.some(c => c.name === command && (!c.agent || c.agent === agentId))) && commandMatches('/', agentId, nativeCommands).some(c => c.name === command)) {
      if (!desktop) { setError('原生命令需要在桌面版运行。'); return; }
        setSending(true);
      try {
        await requireChatConnection(agentId, connectionId, sessionId);
        let id = sessionId;
        if (!id) { const s = await createChatSession({ projectId, model, agent: agentId, connectionId }); id = s.id; setSessionId(id); current.current.sessionId = id; }
        pendingSession.current = id;
        await call('send_message', { sessionId: id, text: `${command}${argument ? ` ${argument}` : ''}`, model: model || null, readOnly, permissionMode }); setInput(''); await refresh();
      } catch (e) { report(e); } finally { pendingSession.current = ''; setSending(false); }
      return;
    }
    if (command === '/compact' && ['opencode', 'pi'].includes(agentId)) {
      if (!sessionId || !desktop) { setError('先发送一条消息，再压缩上下文。'); return; }
      setSending(true);
      try { await call('compact_session_context', { sessionId }); setInput(''); await refresh(); await loadMessages(sessionId); }
      catch (e) { report(e); } finally { setSending(false); }
      return;
    }
    if (!['/goal', '/goal-status', '/compact'].includes(command) || agentId !== 'codex') { setError('当前 Agent 不支持此命令。输入 / 查看可用命令。'); return; }
    if (!desktop) { setError('原生命令需要在桌面版运行。'); return; }
    setSending(true);
    try {
      await requireChatConnection(agentId, connectionId, sessionId);
      let id = sessionId;
      if (!id) { const s = await createChatSession({ projectId, model, agent: agentId, connectionId }); id = s.id; setSessionId(id); current.current.sessionId = id; }
      pendingSession.current = id;
      const result = await call<{ text: string; session: Session }>('execute_agent_command', { sessionId: id, command: command.slice(1), argument, permissionMode });
      current.current.nativeId = result.session.nativeId; setCommandMessage(result.text); setInput(''); await refresh(); await loadMessages(id);
    } catch (e) { report(e); } finally { pendingSession.current = ''; setSending(false); }
  }
  async function connectionUpdated(_resetAgent?: string) {
    const data = await refresh();
    setConnectionRevision(value => value + 1);
    const defaultFor = (agent: string) => data.profiles.find(p => p.agent === agent && p.current)?.id ?? (data.officialAgents.includes(agent) ? '@official' : '@local');
    const nextDefault = defaultFor(agentId);
    if (!sessionId && nextDefault !== connectionId && (!draftConnectionId || draftConnectionId === '@local' || _resetAgent === agentId && providerRequiredAgent === agentId)) {
      setDraftConnectionId(undefined); draftModels.current.delete(draftKey.current);
      modelRequest.current++; setModels([]); setModelSource(undefined); setModel('');
    }
    // A blocked side draft has no conversation binding yet. Let an explicitly
    // added default replace its hidden fallback while preserving the saved text.
    setSideDraft(old => {
      if (!old) return old;
      const agent = old.agent ?? agentId, next = defaultFor(agent);
      if (next === '@local' || next === old.connectionId || !(old.connectionId === '@local' || _resetAgent === agent && providerRequiredAgent === agent)) return old;
      return { ...old, connectionId: next, payload: old.payload ? { ...old.payload, model: data.profiles.find(p=>p.id===next)?.model ?? null } : old.payload };
    });
  }
  async function chooseModel(option: ModelOption) {
    if (anyBusy || configuring) return;
    if (option.connectionId === connectionId && option.model === model) return;
    setConfiguring(true);
    setHandoffTarget(option.source.providerName || option.displayName);
    modelRequest.current++;
    setLoadingModels(false);
    try {
      if (!sessionId) {
        setDraftConnectionId(option.connectionId); setModel(option.model); setModelSource(option.source); setModels([]); setUsage(undefined);
        setEffort(option.defaultReasoningEffort ?? '');
        draftModels.current.set(draftKey.current, { connectionId: option.connectionId, model: option.model, effort: option.defaultReasoningEffort ?? '' });
        return;
      }
      const id = sessionId;
      const saved = await call<Session>('switch_session_model', { sessionId: id, connectionId: option.connectionId, model: option.model });
      if (current.current.sessionId === id) {
        current.current.nativeId = saved.nativeId;
        setModel(saved.model ?? option.model); setModels([]); setModelSource(option.source); setUsage(undefined);
        setEffort(option.defaultReasoningEffort ?? '');
        await loadMessages(id);
      }
      await refresh();
    } catch (e) { report(e); await refresh(); }
    finally { setConfiguring(false); setHandoffTarget(''); }
  }
  async function loadModels() {
    setLoadingModels(true);
    const requestedAgent = agentId;
    const request = ++modelRequest.current;
    try { const result = await call<ModelCatalog>('list_models', { agent: requestedAgent, sessionId: sessionId || null, connectionId }); if (request === modelRequest.current) { setModels(result.data.filter(m => !('hidden' in m) || !m.hidden)); setModelSource(result.source); setModelSourceRevision(profileRevision); setModel(previous => previous || result.data.find(m => m.isDefault)?.model || ''); } }
    catch (e) { if (request === modelRequest.current) report(e); } finally { if (request === modelRequest.current) setLoadingModels(false); }
  }
  async function stop() { setStopping(true); try { await call('interrupt_turn', { sessionId }); } catch (e) { report(e); } finally { setStopping(false); } }
  const showFile = useCallback(async (path: string, line?: number, workspaceSessionId?: string) => {
    const id = workspaceSessionId ? '' : projectId;
    const owner = workspaceSessionId || sessionId;
    try {
      if (systemDocument(path) && !/\.(txt|md|csv|html?)$/i.test(path)) { await call('open_project_path', { projectId: id, sessionId: owner || null, path, action: 'open' }); return; }
      const text = await call<string>('read_project_file', { projectId: id, sessionId: owner || null, path }); if (current.current.sessionId === sessionId) { setPreview({ name: path, text, line }); if (workspaceSessionId) setSideDraft(null); if (narrow) setContextOverlay(true); else setContext(true); }
    } catch (e) { if (current.current.sessionId === sessionId) report(e); }
  }, [projectId, sessionId, narrow]);
  const openPath = useCallback(async (path: string, action: 'open' | 'reveal') => { await call('open_project_path', { projectId, sessionId: sessionId || null, path, action }); }, [projectId, sessionId]);
  function reviewFile(path: string) { const file = diffFiles(changes.diff).find(f => f.path === path); if (file) showDiff(path, file.diff); else void showFile(path); }
  function selectCommand(index: number) { const command = slashOptions[index]; if (!command) return; if (command.skill) { if (!addSkill(command.skill)) return; setInput(''); } else setInput(`${command.name}${command.arguments ? ' ' : ''}`); setCommandsDismissed(true); textarea.current?.focus(); }
  function switchAgent(value: string) {
    selectProject(project, value);
    setLoadingModels(false);
  }
  async function inspectRuntime() { try { setRuntime(await call<RuntimeInfo>('runtime_info')); } catch (e) { report(e); } }
  async function saveCodex() {
    setConfiguring(true);
    try { await call('configure_codex', { path: codexPath.trim() || null, loadMcp }); setModels([]); await refresh(); await inspectRuntime(); }
    catch (e) { report(e); } finally { setConfiguring(false); }
  }
  async function archive() {
    if (!session || modalWorking) return;
    setModalWorking(true);
    try { await call('archive_session', { sessionId }); await refresh(); selectProject(project); setModal(null); } catch (e) { report(e); } finally { setModalWorking(false); }
  }
  async function saveRename() {
    if (modalWorking || !rename.trim()) return;
    setModalWorking(true); setModalError('');
    try { await call('rename_session', { sessionId, title: rename.trim() }); await refresh(); setModal(null); } catch (e) { report(e); } finally { setModalWorking(false); }
  }
  async function reconcileSidebar() {
    const data = await refresh();
    const active = current.current;
    const next = data.sessions.find(s => s.id === active.sessionId);
    if (next && (next.connectionId !== sessions.find(s => s.id === active.sessionId)?.connectionId || next.model !== sessions.find(s => s.id === active.sessionId)?.model)) { setModel(next.model ?? ''); setModels([]); setUsage(undefined); await loadMessages(next.id); }
    if (next && (next.projectId !== active.projectId || next.nativeId !== active.nativeId)) {
      current.current = { sessionId: next.id, projectId: next.projectId, nativeId: next.nativeId };
      setProjectId(next.projectId);
      void refreshChanges(next.projectId);
    } else if (active.sessionId && !next || active.projectId && !data.projects.some(p => p.id === active.projectId)) {
      const p = data.projects.find(p => p.id === active.projectId);
      selectProject(p);
    }
    if(next && current.current.sessionId===next.id)await loadMessages(next.id);
  }
  sidebarUpdateRef.current = reconcileSidebar;
  const shownProjects = projects.filter(p => p.name.toLowerCase().includes(search.toLowerCase()) || sessions.some(s => s.projectId === p.id && s.title.toLowerCase().includes(search.toLowerCase())));
  function addSkill(skill: Skill) { if (skill.error) { report(skill.error); return false; } if (attachments.length >= 12 && !attachments.some(a => a.path === skill.path)) { report('最多添加 12 个附件，请先移除一个附件。'); return false; } setAttachments(old => old.some(a => a.path === skill.path) ? old : [...old, { kind: 'skill' as const, name: skill.name, path: skill.path }]); setModal(null); setSettingsOpen(false); requestAnimationFrame(() => textarea.current?.focus()); return true; }

  function openSettings() {
    if (!settingsOpenRef.current) settingsScroll.current = scrollArea.current?.scrollTop ?? 0;
    setModal(null); setSettingsOpen(true);
  }
  async function navigate(direction: -1 | 1) {
    if (navigating.current || modalRef.current || sidebarDialog) return;
    if (direction === -1 && settingsOpenRef.current) { setSettingsOpen(false); return; }
    let next = travel(navigation, direction);
    while (next !== navigation) {
      const target = next.entries[next.index];
      const exists = target.page === 'settings' || (target.sessionId ? sessions.some(s => s.id === target.sessionId) : !target.projectId || projects.some(p => p.id === target.projectId));
      if (exists) break;
      const further = travel(next, direction); if (further === next) return; next = further;
    }
    if (next === navigation) return;
    const target = next.entries[next.index];
    navigating.current = true;
    try {
      setNavigation(next);
      if (target.page === 'settings') { setSettingsTab(target.tab as typeof settingsTab); openSettings(); }
      else if (target.sessionId && target.sessionId !== sessionId) await selectSession(sessions.find(s => s.id === target.sessionId)!);
      else if (target.projectId !== projectId || target.sessionId !== sessionId) { const p = projects.find(p => p.id === target.projectId); selectProject(p); }
      else setSettingsOpen(false);
    } finally { navigating.current = false; }
  }
  navigationAction.current = direction => { void navigate(direction); };
  async function edit(command: 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'selectAll') {
    const target = editTarget.current;
    if (target?.isConnected && target.offsetParent !== null) target.focus({ preventScroll: true });
    try {
      if (command === 'paste') {
        const images = target === textarea.current ? await readClipboardImages().catch(() => []) : [];
        if (images.length) await addAttachments(images);
        else document.execCommand('insertText', false, await navigator.clipboard.readText());
      }
      else document.execCommand(command);
    } catch (e) { report(e); }
  }
  function addAttachments(items: (Attachment | File)[]): Promise<void> {
    if (!desktop) return Promise.reject(new Error('请在桌面版中上传或粘贴图片'));
    if (!items.length) return Promise.resolve();
    const origin = draftKey.current;
    drafts.current.set(origin, inputRef.current);
    pendingAttachments.current++; setImportingImages(true);
    const job = attachmentJobs.current.then(async () => {
      const currentAttachments = () => draftKey.current === origin ? attachmentsRef.current : attachmentDrafts.current.get(origin) ?? [];
      if (items.length + currentAttachments().length > MAX_ATTACHMENTS) throw new Error('最多添加 12 个附件，请先移除一个附件');
      for (const item of items) if (item instanceof File) validateImageFile(item);
      const imported: Attachment[] = [];
      for (const item of items) {
        if (!(item instanceof File) && item.kind !== 'image') { imported.push(item); continue; }
        const remainingBytes = MAX_TOTAL_IMAGE_BYTES - imageBytes(mergeAttachments(currentAttachments(), imported));
        const args = item instanceof File ? { data: await imageBase64(item), name: item.name, remainingBytes } : { path: item.path, name: item.name, remainingBytes };
        imported.push(await call<Attachment>('import_image_attachment', args));
      }
      const next = mergeAttachments(currentAttachments(), imported);
      attachmentDrafts.current.set(origin, next);
      if (draftKey.current === origin) { attachmentsRef.current = next; setAttachments(next); }
      // Imports can finish after the user has switched conversations. Keep them
      // with their original draft, including when the app is closed afterwards.
      try { localStorage.setItem('supercode.drafts.v1', encodeDrafts(drafts.current, attachmentDrafts.current, draftModels.current)); } catch { /* In-memory drafts remain available. */ }
    }).finally(() => { pendingAttachments.current--; setImportingImages(pendingAttachments.current > 0); });
    attachmentJobs.current = job.catch(() => {});
    return job;
  }
  function pasteImages(event: React.ClipboardEvent<HTMLTextAreaElement>) {
    const images = clipboardImages(event.clipboardData);
    if (!images.length) return;
    event.preventDefault();
    const text = event.clipboardData.getData('text/plain');
    if (text) {
      const start = event.currentTarget.selectionStart; const end = event.currentTarget.selectionEnd;
      setInput(value => value.slice(0, start) + text + value.slice(end));
      requestAnimationFrame(() => textarea.current?.setSelectionRange(start + text.length, start + text.length));
    }
    void addAttachments(images).catch(report);
  }
  const desktopMenus: DesktopMenu[] = [
    { label: '文件', items: [
      { label: '新建聊天', shortcut: 'Ctrl+N', run: () => void newSession(), disabled: !ready },
      { label: '新窗口', run: () => void call('new_workspace_window').catch(report), disabled: !desktop || !ready },
      { label: '添加项目…', run: () => void addProject(undefined, desktop), disabled: !ready },
      { label: '搜索会话…', shortcut: 'Ctrl+K', run: () => setModal('search') },
      { label: '设置', shortcut: 'Ctrl+,', divider: true, run: () => { setSettingsTab('general'); openSettings(); } },
      { label: '退出', divider: true, run: () => void call('request_app_exit').catch(report), disabled: !desktop },
    ] },
    { label: '编辑', items: [
      { label: '撤销', shortcut: 'Ctrl+Z', run: () => void edit('undo') },
      { label: '重做', shortcut: 'Ctrl+Y', run: () => void edit('redo') },
      { label: '剪切', shortcut: 'Ctrl+X', divider: true, run: () => void edit('cut') },
      { label: '复制', shortcut: 'Ctrl+C', run: () => void edit('copy') },
      { label: '粘贴', shortcut: 'Ctrl+V', run: () => void edit('paste') },
      { label: '全选', shortcut: 'Ctrl+A', divider: true, run: () => void edit('selectAll') },
    ] },
    { label: '视图', items: [
      { label: '显示侧栏', shortcut: 'Ctrl+B', checked: sidebar, run: () => setSidebar(v => !v) },
      { label: '文件变更', checked: showContext, disabled: settingsOpen, run: () => narrow ? setContextOverlay(v => !v) : setContext(v => !v) },
      { label: '运行日志', checked: logsOpen, disabled: settingsOpen, run: () => setLogsOpen(v => !v) },
      { label: '外观', divider: true, run: () => { setSettingsTab('appearance'); openSettings(); } },
      { label: '资源与进程', run: () => { setSettingsTab('resources'); openSettings(); } },
    ] },
    { label: '帮助', items: [
      { label: '快捷键', run: () => { setSettingsTab('general'); openSettings(); } },
      { label: '关于 SuperCode', run: () => { setSettingsTab('about'); openSettings(); } },
    ] },
  ];

  return <div className="desktop-shell"><DesktopTitleBar menus={desktopMenus} back={() => void navigate(-1)} forward={() => void navigate(1)} canBack={settingsOpen || navigation.index > 0} canForward={navigation.index < navigation.entries.length - 1} sidebar={sidebar} toggleSidebar={() => setSidebar(v => !v)} blocked={!!modal || sidebarDialog} report={report}/><div className="desktop-workarea"><ActivityRail settings={settingsOpen} blocked={!!modal || sidebarDialog} home={() => setSettingsOpen(false)} create={() => { setSettingsOpen(false); void newSession(); }} search={() => setModal('search')} projects={() => { setSettingsOpen(false); setSidebar(true); }} preferences={() => { setSettingsTab('general'); openSettings(); }}/><div ref={panels.root} className={`app ${settingsOpen ? 'settings-open' : ''} ${sidebar ? '' : 'sidebar-hidden'} ${showContext || sideDraft ? '' : 'context-hidden'} ${sideDraft?'has-side-chat':''}`}>
    {sidebar ? <aside className="sidebar" hidden={settingsOpen} inert={!!modal || sidebarDialog}>
      {!settingsOpen ? <div {...panels.handle('left')}/> : null}
      <div className="brand"><span>SuperCode</span><button className="icon-button brand-search" aria-label="搜索会话" title="搜索会话 (Ctrl+K)" onClick={() => setModal('search')}><Search size={16}/></button></div>
      <button className="new-chat" title="新建聊天 (Ctrl+N)" onClick={() => void newSession()}><SquarePen size={15} />新聊天</button>
      <Sidebar projects={projects} sessions={sessions} state={sidebarState} projectId={projectId} sessionId={sessionId} ready={ready} blocked={!!modal || settingsOpen} collapsed={collapsedProjects} setCollapsed={setCollapsedProjects} selectProject={selectProject} selectSession={selectSession} addProject={() => void addProject()} createSession={newSession} updated={reconcileSidebar} dialogChanged={setSidebarDialog}/>
    </aside> : null}

    <main className={`workspace${newChat ? ' new-chat-page' : ''}`} hidden={settingsOpen} inert={!!modal || sidebarDialog}>
      {desktop ? <AppUpdateNotice open={() => { setSettingsTab('about'); openSettings(); }}/> : null}
      <header className="workspace-header"><div className="breadcrumbs">{!sidebar ? <button className="icon-button" title="展开侧栏" onClick={() => setSidebar(true)}><PanelLeftOpen size={18} /></button> : null}<Folder size={15} /><strong title={`${project?.name ?? '工作区'} · ${session?.title ?? '新会话'}`}>{session?.title ?? project?.name ?? '新聊天'}</strong>{!desktop ? <span className="preview-badge">界面预览</span> : null}</div><div className="header-actions">{session ? <button className="icon-button" title="会话设置" onClick={() => { setRename(session.title); setModal('rename'); }}><MoreHorizontal size={18} /></button> : null}<button className={`icon-button ${logsOpen ? 'on' : ''}`} title="运行日志" onClick={() => setLogsOpen(v => !v)}><Terminal size={16} /></button><button className="icon-button" title={showContext ? '收起变更面板' : '展开变更面板'} onClick={() => narrow ? setContextOverlay(v => !v) : setContext(v => !v)}>{showContext ? <PanelRightClose size={17} /> : <PanelRightOpen size={17} />}</button></div></header>
      <div className="chat-scroll" aria-busy={!ready} ref={scrollArea} onClickCapture={e => { if ((e.target as Element).closest('.activity-row, .activity-group-summary, .activity-toggle-more')) stickToBottom.current = false; }} onScroll={() => { if (settingsOpenRef.current) return; const el = scrollArea.current; if (el) { stickToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 100; setShowJump(!stickToBottom.current); } }}>
        {!ready ? <div className="startup-loading" role="status"><span className="pending-text">正在加载工作区…</span></div> : messages.length || busy || messagesLoading || currentRequests.length ? <div className="messages" aria-busy={messagesLoading}>{hasMore ? <button className="load-earlier" disabled={messagesLoading} onClick={() => void loadMessages(sessionId, messages.find(m => m.seq > 0)?.seq).catch(report)}>{messagesLoading ? '正在加载…' : '加载更早的消息'}</button> : null}{messagesLoading && !messages.length ? <div className="working-indicator"><span className="pending-text">正在加载会话…</span></div> : null}<Conversation messages={messages} agentName={agentName} autoExpand={autoExpand} showDiff={showDiff} openFile={showFile} openPath={openPath} projectId={projectId} sessionId={sessionId} />{handoffTarget ? <div className="working-indicator" role="status"><span className="pending-text">{handoffTarget === '@compact' ? '正在压缩上下文…' : `正在切换到 ${handoffTarget}…`}</span></div> : busy && !latest ? <div className="working-indicator"><span className="pending-text">{session?.status === 'starting' ? '正在启动 Agent…' : '正在思考…'}</span></div> : null}{currentRequests.filter(request => request.method !== 'item/tool/requestUserInput').map(request => <Suspense key={String(request.id)} fallback={<div className="working-indicator">正在加载确认请求…</div>}><RequestCard request={request} respond={result => respondRequest(request, result)} /></Suspense>)}</div>
          : <NewChat project={project} projects={projects} selectProject={selectProject} addProject={() => void addProject()}/>}
      </div>
      {showJump ? <JumpToLatest onClick={() => { stickToBottom.current = true; setShowJump(false); scrollToLatest(scrollArea.current, !prefs.animations || window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth'); }} /> : null}
      <div className={`composer-area${questionPending ? ' has-question' : ''}`}>{error ? <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" title="关闭提示" onClick={() => setError('')}><X size={15} /></button></div> : null}{activeElsewhere && !busy ? <button className="active-elsewhere" onClick={() => void selectSession(activeElsewhere)}><span className="pending-text">另一会话正在运行：{activeElsewhere.title}</span><ArrowUpRight size={13} /></button> : null}
        {commandMessage ? <div className="command-feedback" role="status"><pre>{commandMessage}</pre><button className="icon-button" title="关闭命令结果" onClick={() => setCommandMessage('')}><X size={14} /></button></div> : null}
        {questionPending ? <Suspense fallback={<div className="working-indicator">正在加载提问…</div>}><QuestionDock requests={currentRequests} respond={respondRequest} /></Suspense> : null}
        <FollowupQueue rows={followups.rows} agent={agentId} turnId={session?.turnId} steeringMode={followups.steeringMode} change={changeFollowup} steer={steerFollowup} openSide={row=>void openSide(row)}/>
        <form className={`composer ${busy ? 'busy' : ''}`} onSubmit={e => { e.preventDefault(); void send(); }}>
          {slashOptions.length || composerFocused && skillQuery && !commandsDismissed && (skillsLoading || skillsError) ? <div className="slash-menu" id="agent-commands" role="listbox" aria-label="命令与技能" aria-busy={skillsLoading}>{slashOptions.map((c, index) => <button id={`agent-command-${index}`} className={index === slashIndex ? 'selected' : ''} type="button" role="option" aria-selected={index === slashIndex} key={c.name} onMouseDown={e => e.preventDefault()} onClick={() => selectCommand(index)}><span className="slash-option-name">{c.skill ? <Sparkles size={14}/> : <Terminal size={14}/>}<code>{c.name}</code></span><span className="slash-option-detail" title={c.detail}>{c.detail}{c.skill ? <small>{skillSource(c.skill)}</small> : null}</span>{c.skill ? <small className="slash-option-kind">技能</small> : null}</button>)}{skillsLoading ? <div className="slash-menu-status" role="status">读取技能…</div> : skillsError ? <div className="slash-menu-status error-text" role="alert">{skillsError}</div> : null}</div> : null}
          <AttachmentStrip attachments={attachments} loading={importingImages} remove={path => setAttachments(old => old.filter(a => a.path !== path))} />
          <div className="composer-input"><ComposerSkills attachments={attachments} remove={path => { setAttachments(old => old.filter(a => a.path !== path)); requestAnimationFrame(() => textarea.current?.focus()); }} /><textarea ref={textarea} disabled={!ready} onFocus={() => { setComposerFocused(true); setCommandsDismissed(false); }} onBlur={() => { setComposerFocused(false); setCommandsDismissed(true); }} value={input} onPaste={pasteImages} onChange={e => { setInput(e.target.value); setCommandsDismissed(false); setCommandSelection(0); }} aria-controls={slashOptions.length ? "agent-commands" : undefined} aria-expanded={!!slashOptions.length} aria-autocomplete="list" aria-activedescendant={slashOptions.length ? `agent-command-${slashIndex}` : undefined} placeholder={questionPending ? '随心输入' : attachments.some(a => a.kind === 'skill') ? '' : project ? '随心输入，输入 / 查看命令与技能' : '描述你的想法，或先选择一个项目…'} aria-label="消息" rows={questionPending ? 1 : 2} onKeyDown={e => { if (e.nativeEvent.isComposing) return; if (e.key === 'Backspace' && !e.repeat && !e.ctrlKey && !e.altKey && !e.metaKey && e.currentTarget.selectionStart === 0 && e.currentTarget.selectionEnd === 0) { const lastSkill = attachments.filter(a => a.kind === 'skill').at(-1); if (lastSkill) { e.preventDefault(); setAttachments(old => old.filter(a => a.path !== lastSkill.path)); return; } } if (slashOptions.length && ['ArrowDown','ArrowUp'].includes(e.key)) { e.preventDefault(); setCommandSelection(commandIndex(e.key, slashIndex, slashOptions.length)); } else if (slashOptions.length && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setCommandsDismissed(true); } else if (slashOptions.length && (e.key === 'Tab' || e.key === 'Enter' && !e.shiftKey)) { e.preventDefault(); selectCommand(slashIndex); } else if (skillsLoading && skillQuery && e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); } else if (e.key === 'Enter' && (prefs.enterSend || e.ctrlKey || e.metaKey) && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void send(); } }} /></div>
          <div className="composer-toolbar"><div className="composer-options">
            <AddMenu addAttachments={addAttachments} busy={sending || importingImages} project={() => void addProject()} openSkills={() => {setSettingsTab('skills');openSettings();}} />
            <PermissionMenu value={permissionMode} agent={agentId} busy={anyBusy} onChange={mode => {setPermissionMode(mode);}} />
          </div><div className="composer-right">
            <AgentMenu value={agentId} agents={agents} busy={anyBusy} choose={switchAgent}/>
            <ModelMenu value={model} models={modelSourceRevision === profileRevision ? models : []} profiles={profiles} activeProfile={activeProfile} connectionId={connectionId} order={connectionOrder} source={currentModelSource} busy={anyBusy || configuring} loading={loadingModels} agent={agentId} choose={chooseModel} reload={() => void loadModels()} manage={() => {setProviderSettingsAgent(agentId);setSettingsTab('providers');openSettings();}} effort={effort} setEffort={setEffort} hasConversation={!!session?.nativeId} />
            {busy ? <button className="send-button stop-button" type="button" disabled={!session?.turnId || stopping} onClick={() => void stop()} aria-label="停止生成"><Square size={14} fill="currentColor" /></button> : null}<button className="send-button" type="submit" disabled={(!input.trim() && !attachments.length) || sending || importingImages} aria-label={anyBusy ? '加入队列' : '发送消息'} title={anyBusy ? '加入队列，当前任务完成后发送' : '发送消息'}><ArrowUp size={18} /></button></div></div>
        </form><div className="composer-footnote"><span /><div>{currentModelSource?.available === true ? <PlatformUsageIndicator key={`${agentId}:${connectionId}`} agent={agentId} connectionId={agentId === 'codex' && connectionId === '@local' ? '@official' : connectionId} revision={profileRevision} openStats={() => {setSettingsTab('quota');openSettings();}} /> : null}<UsageIndicator usage={usage} busy={anyBusy || !session?.nativeId} compact={() => void handleCommand('/compact','')} openStats={() => {setSettingsTab('usage');openSettings();}} /><span>{prefs.enterSend ? 'Enter 发送' : 'Ctrl Enter 发送'} · Shift Enter 换行</span></div></div>
      </div>
      {logsOpen ? <section className="logs-panel"><div><span><Terminal size={14} />运行日志</span><button className="quiet-button" onClick={() => setLogs([])}>清空</button><button className="icon-button" title="关闭日志" onClick={() => setLogsOpen(false)}><X size={15} /></button></div><pre>{logs.length ? logs.join('\n') : '暂无运行日志。Agent 会在发送消息时启动。'}</pre></section> : null}
    </main>

    <SelectionActions key={sessionId} area={scrollArea} add={text=>{setInput(old=>[old,text.split('\n').map(line=>`> ${line}`).join('\n')].filter(Boolean).join('\n\n'));requestAnimationFrame(()=>textarea.current?.focus());}} details={setSelectionDetail} ask={text=>void openSide(undefined,`关于下面这段内容：\n\n${text}\n\n请帮我进一步解释。`)}/>
    {sideDraft && !settingsOpen ? <Suspense fallback={<aside className="side-chat">正在打开侧边聊天…</aside>}><SideChat key={sideDraft.key} draft={sideDraft} projectId={sideDraft.projectId??projectId} agent={sideDraft.agent??agentId} connectionId={sideDraft.connectionId??connectionId} defaults={followupPayload('')} close={()=>setSideDraft(null)} opened={()=>void refresh()} openFile={showFile} showDiff={showDiff} providerRequired={(agent,draft)=>{setSideDraft(draft);showProviderRequired(agent);}} resizeHandle={<div {...panels.handle('right')}/>}/></Suspense> : null}
    {selectionDetail?<div className="modal-backdrop" onClick={()=>setSelectionDetail('')}><section className="modal" role="dialog" aria-modal="true" aria-label="选中内容" onClick={e=>e.stopPropagation()}><div className="modal-heading"><h2>选中内容</h2><button className="icon-button" aria-label="关闭选中内容" onClick={()=>setSelectionDetail('')}><X size={16}/></button></div><pre className="selection-detail">{selectionDetail}</pre><CopyButton text={selectionDetail} label="复制选中内容"/></section></div>:null}
    {showContext && !sideDraft ? <aside className="context-panel" hidden={settingsOpen} inert={!!modal || sidebarDialog}>{!settingsOpen ? <div {...panels.handle('right')}/> : null}<div className="context-header"><span title={workspacePath}>工作区</span>{workspacePath && !project ? <button className="icon-button" title="定位聊天目录" onClick={() => void openPath('.', 'reveal').catch(report)}><Folder size={15}/></button> : null}{narrow ? <button className="icon-button" title="关闭变更面板" onClick={() => setContextOverlay(false)}><X size={16} /></button> : null}<button className={`icon-button ${changesLoading ? 'spin' : ''}`} title="刷新变更" disabled={changesLoading || !workspacePath} onClick={() => void refreshChanges(projectId)}><RefreshCw size={15} /></button></div><div className="context-tabs"><span className="active">文件变更<small>{changes.files.length}</small></span>{changes.branch ? <span className="branch"><GitBranch size={12} />{changes.branch}</span> : null}</div>{changesLoading && !changes.files.length && !preview ? <div className="context-empty"><LoaderCircle size={22} className="spin" /><strong>正在检查文件变更…</strong></div> : preview ? <div className="file-preview"><div className="file-preview-header"><FileCode2 size={14} /><span title={preview.name}>{preview.name}</span><CopyButton text={preview.text} label="复制文件内容" iconOnly /><button className="icon-button" title="关闭文件" onClick={() => setPreview(null)}><X size={14} /></button></div><Suspense fallback={<pre>{preview.text}</pre>}>{preview.diff ? <DiffView text={preview.text} /> : <SourceView text={preview.text} line={preview.line} />}</Suspense></div> : changes.files.length ? <><div className="changed-files">{changes.files.map(f => <button key={f.path} disabled={f.path.endsWith("/")} onClick={() => reviewFile(f.path)} title={f.path.endsWith("/") ? `${f.path} · 未跟踪文件夹` : f.path}><FileCode2 size={15} /><span>{f.path}</span><small className={f.status === '??' ? 'untracked' : ''}>{f.status === '??' ? 'U' : f.status}</small></button>)}</div>{changes.diff ? <button className="workspace-diff-button" onClick={() => showDiff('工作区变更', changes.diff)}>查看全部差异</button> : null}</> : <div className="context-empty"><span><FileCode2 size={24} /></span><strong>{!workspacePath ? '聊天文件' : !project ? '聊天工作目录'  : changes.isGit ? '暂无文件变更' : '未启用 Git'}</strong><p>{!workspacePath ? '发送消息后自动创建独立目录，也可以选择项目' : !project ? workspacePath  : changes.isGit ? '文件修改后会显示在这里' : '初始化 Git 后可查看文件差异'}</p></div>}</aside> : null}

    {settingsOpen ? <Suspense fallback={<div className="startup-loading" role="status">正在加载设置…</div>}><SettingsPage tab={settingsTab} select={setSettingsTab} blocked={!!modal || sidebarDialog}>
      {modalError ? <div className="error-banner modal-error" role="alert"><span>{modalError}</span><button className="icon-button" aria-label="关闭错误提示" onClick={() => setModalError('')}><X size={14} /></button></div> : null}
      {settingsTab === 'agents' ? <AgentSettings agents={agents} busy={anyBusy || configuring} loadMcp={loadMcp} setLoadMcp={setLoadMcp} saveMcp={saveCodex}/> : null}
      {settingsTab === 'providers' ? <Suspense fallback={<p className="muted">正在加载模型连接…</p>}><ProviderSettings profiles={profiles} officialAgents={officialAgents} order={connectionOrder} busy={anyBusy} updated={connectionUpdated} initialAgent={providerSettingsAgent} /></Suspense> : null}
      {settingsTab === 'general' ? <GeneralSettings prefs={prefs} setPrefs={setPrefs} busy={anyBusy} autoExpand={autoExpand} setAutoExpand={setAutoExpand} agents={agents} /> : null}
      {settingsTab === 'usage' ? <Suspense fallback={<p className="muted">加载中…</p>}><UsagePanel /></Suspense> : null}
      {settingsTab === 'quota' ? <Suspense fallback={<p className="muted">加载中…</p>}><QuotaPanel profiles={profiles} officialAgents={officialAgents} /></Suspense> : null}
      {settingsTab === 'about' ? <div className="about-settings"><div className="settings-page-heading"><h2>关于</h2></div><div className="about-app"><img className="app-brand-icon" src="/app-icon.png" alt="SuperCode 图标" width={56} height={56} /><div><strong>SuperCode</strong><span>当前版本 {appVersion}</span></div></div><AppUpdateSettings busy={anyBusy}/></div> : null}
      {['appearance','personalization','notifications','skills','plugins','automation'].includes(settingsTab) ? <Suspense fallback={<p className="muted">加载中…</p>}><ClientSettings key={settingsTab} tab={settingsTab as ClientTab} prefs={prefs} setPrefs={setPrefs} projectId={projectId} busy={anyBusy} addSkill={addSkill} /></Suspense> : null}
{settingsTab === 'resources' ? <><div className="settings-page-heading"><h2>资源</h2><button className="icon-button" title="刷新内存" onClick={() => void inspectRuntime()}><RefreshCw size={14} /></button></div><div className="settings-section">{runtime && desktop ? <><div className="memory-grid"><div><span>应用与 WebView</span><strong>{(runtime.shellBytes / 1024 / 1024).toFixed(0)}<small> MiB</small></strong></div><div><span>Agent 与工具</span><strong>{(runtime.agentBytes / 1024 / 1024).toFixed(0)}<small> MiB</small></strong></div></div><p className="muted">{runtime.running ? 'Agent 运行中' : 'Agent 未运行'}</p></> : <p className="muted">{desktop ? '正在读取内存…' : '桌面版可查看内存'}</p>}<dl className="resource-info">{runtime && desktop ? <div><dt>空闲释放</dt><dd>{runtime.idleReleaseSeconds % 60 === 0 ? `${runtime.idleReleaseSeconds / 60} 分钟` : `${runtime.idleReleaseSeconds} 秒`}</dd></div> : null}<div><dt>会话保存</dt><dd>本机</dd></div></dl>{desktop ? <button className="quiet-button" disabled={anyBusy} onClick={() => { void call('release_runtime').then(inspectRuntime).catch(report); }}>释放空闲进程</button> : null}</div></> : null}
    </SettingsPage></Suspense> : null}

    {modal ? <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget && !modalWorking) setModal(null); }}><section className="modal" ref={modalElement} role="dialog" aria-modal="true" aria-label={modal === 'search' ? '搜索会话' : modal === 'rename' ? '会话设置' : modal === 'provider' ? '添加模型供应商' : '添加项目'}><div className="modal-header"><h2>{modal === 'search' ? '搜索会话' : modal === 'rename' ? '会话设置' : modal === 'provider' ? '添加模型供应商' : '添加项目'}</h2><button className="icon-button" title="关闭" disabled={modalWorking} onClick={() => setModal(null)}><X size={18}/></button></div>
      {modal === 'provider' ? <><p>{agents.find(a=>a.id===providerRequiredAgent)?.name ?? (({claude:'Claude Code',codex:'Codex',opencode:'OpenCode',pi:'Pi'} as Record<string,string>)[providerRequiredAgent] ?? providerRequiredAgent)} 当前对话没有可用的连接。请先添加模型供应商，再发送消息。</p><div className="modal-actions"><button type="button" className="quiet-button" onClick={()=>setModal(null)}>取消</button><button type="button" className="primary-button" onClick={()=>{setProviderSettingsAgent(providerRequiredAgent);setSettingsTab('providers');openSettings();}}>去添加</button></div></> : null}
      {modalError ? <div className="error-banner modal-error" role="alert">{modalError}</div> : null}
      {modal === 'project' ? <form onSubmit={e => { e.preventDefault(); void addProject(pathInput.trim()); }}><label htmlFor="project-path">项目文件夹路径</label><input id="project-path" autoFocus value={pathInput} onChange={e => setPathInput(e.target.value)} placeholder="D:\Projects\my-project" /><p className="muted">{desktop ? '输入已有项目的完整路径，也可以通过系统对话框选择文件夹。' : '这是界面预览。路径仅用于展示，不会读取本地文件。'}</p><div className="modal-actions">{desktop ? <button type="button" className="quiet-button" onClick={() => void addProject(undefined, true)}>选择文件夹</button> : null}<button className="primary-button" disabled={!pathInput.trim()}>添加项目</button></div></form> : null}
      {modal === 'search' ? <><div className="search-field"><Search size={17} /><input autoFocus value={search} onChange={e => setSearch(e.target.value)} placeholder="搜索项目或会话…" aria-label="搜索项目或会话" /></div><div className="search-results">{shownProjects.map(p => <div key={p.id}><button onClick={() => { selectProject(p); setModal(null); }}><Folder size={16} /><strong>{p.name}</strong></button>{sessions.filter(s => s.projectId === p.id && s.title.toLowerCase().includes(search.toLowerCase())).map(s => <button key={s.id} onClick={() => void selectSession(s)}><MessageSquare size={15} /><span>{s.title}</span></button>)}</div>)}{sessions.filter(s => !s.projectId && s.title.toLowerCase().includes(search.toLowerCase())).map(s => <button key={s.id} onClick={() => void selectSession(s)}><MessageSquare size={15}/><span>{s.title}</span></button>)}{!shownProjects.length && !sessions.some(s => !s.projectId && s.title.toLowerCase().includes(search.toLowerCase())) ? <p className="muted">没有找到匹配的会话。</p> : null}</div></> : null}
      {modal === 'rename' ? <><form onSubmit={e => { e.preventDefault(); void saveRename(); }}><label htmlFor="rename">会话名称</label><input id="rename" autoFocus value={rename} onChange={e => setRename(e.target.value)} maxLength={100} /><div className="modal-actions"><button type="button" className="quiet-button" disabled={busy || modalWorking} onClick={() => void archive()}>归档会话</button><button className="primary-button" disabled={!rename.trim() || modalWorking}>{modalWorking ? '保存中…' : '保存'}</button></div></form></> : null}
    </section></div> : null}
  </div></div></div>;
}
