import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Bootstrap, Project, Session, Message, RpcEvent } from './types';
import { emptySidebar } from './sidebarState';

export const desktop = isTauri();
const sidebarQa = typeof location !== 'undefined' && location.pathname.endsWith('/sidebar-qa.html');
const previewKey = sidebarQa ? 'supercode.sidebar.qa.v1' : typeof location !== 'undefined' && location.pathname.endsWith('/sidebar-status-qa.html') ? 'supercode.sidebar.status.qa.v1' : 'supercode.preview.v1';
const archiveKey = `${previewKey}.archives`;
const removedProjectsKey = `${previewKey}.removedProjects`;
const defaultPreview: Bootstrap = {
  projects: [{ id: 'preview', name: 'SuperCode', path: '界面预览 · 项目内容仅保存在浏览器' }],
  sessions: [],
  profiles: [],
  officialAgents: [],
  sidebar: emptySidebar,
  codexPath: null,
  loadMcp: false,
  agents: [
    { id: 'codex', name: 'Codex', installed: false, connected: true, path: null },
    { id: 'claude', name: 'Claude Code', installed: false, connected: true, path: null },
    { id: 'opencode', name: 'OpenCode', installed: false, connected: false, path: null },
    { id: 'pi', name: 'Pi', installed: false, connected: false, path: null },
  ],
};
function readPreview(): Bootstrap {
  try { const saved = JSON.parse(localStorage.getItem(previewKey) ?? 'null'); const base = structuredClone(defaultPreview); return saved?.projects && saved?.sessions ? { ...base, ...saved, agents: base.agents } : base; }
  catch { return structuredClone(defaultPreview); }
}
function writePreview(data: Bootstrap) { localStorage.setItem(previewKey, JSON.stringify(data)); }
export async function call<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (desktop) return invoke<T>(command, args);
  const data = readPreview();
  data.sidebar ??= structuredClone(emptySidebar);
  let result: unknown;
  switch (command) {
    case 'bootstrap': result = data; break;
    case 'add_project': {
      const path = String(args.path).trim(); if (!path) throw new Error('请输入项目路径');
      const project: Project = { id: crypto.randomUUID(), name: path.split(/[\\/]/).filter(Boolean).pop() ?? path, path };
      data.projects.push(project); writePreview(data); result = project; break;
    }
    case 'create_session': {
      const session: Session = { id: crypto.randomUUID(), projectId: String(args.projectId), title: '新会话', agent: String(args.agent ?? 'codex'), model: args.model ? String(args.model) : null, nativeId: null, status: 'idle', updatedAt: Date.now() / 1000, turnId: null, connectionId: args.connectionId ? String(args.connectionId) : undefined };
      data.sessions.unshift(session); writePreview(data); result = session; break;
    }
    case 'switch_session_model': {
      const session = data.sessions.find(s => s.id === args.sessionId);
      if (!session) throw new Error('会话不存在');
      session.connectionId = String(args.connectionId || '@local'); session.model = String(args.model); session.nativeId = null;
      writePreview(data); result = session; break;
    }
    case 'list_followups': result=[];break;
    case 'followup_capabilities': result={steeringMode:'native',queue:'client',editableUntilSubmitted:true};break;
    case 'list_messages': result = [] as Message[]; break;
    case 'workspace_changes': result = { isGit: false, branch: '', files: [], diff: '' }; break;
    case 'rename_session': data.sessions = data.sessions.map(s => s.id === args.sessionId ? { ...s, title: String(args.title) } : s); writePreview(data); break;
    case 'archive_session': {
      const archives = JSON.parse(localStorage.getItem(archiveKey) ?? '[]') as Session[];
      const session = data.sessions.find(s => s.id === args.sessionId);
      if (session) archives.push(session);
      localStorage.setItem(archiveKey, JSON.stringify(archives));
      data.sessions = data.sessions.filter(s => s.id !== args.sessionId); writePreview(data); break;
    }
    case 'get_sidebar_state': result = data.sidebar; break;
    case 'mark_session_read': {
      const session = data.sessions.find(s => s.id === args.sessionId);
      const item = data.sidebar.sessions[String(args.sessionId)];
      result = !!item?.unread && !!session && !['starting', 'running', 'waiting'].includes(session.status);
      if (result) { item.unread = false; writePreview(data); }
      break;
    }
    case 'update_sidebar_item': {
      const list = args.kind === 'project' ? data.sidebar.projects : data.sidebar.sessions;
      const item = list[String(args.id)] ??= { pinned: false, unread: false, sectionId: null };
      if (args.action === 'pin') item.pinned = Boolean(args.value);
      if (args.action === 'unread') item.unread = Boolean(args.value);
      if (args.action === 'section') item.sectionId = args.value ? String(args.value) : null;
      writePreview(data); break;
    }
    case 'save_sidebar_section': {
      const section = { id: args.id ? String(args.id) : crypto.randomUUID(), name: String(args.name).trim() };
      if (!section.name) throw new Error('分区名称不能为空');
      data.sidebar.sections = [...data.sidebar.sections.filter(s => s.id !== section.id), section]; result = section; writePreview(data); break;
    }
    case 'delete_sidebar_section': {
      data.sidebar.sections = data.sidebar.sections.filter(s => s.id !== args.id);
      [...Object.values(data.sidebar.projects), ...Object.values(data.sidebar.sessions)].forEach(item => { if (item.sectionId === args.id) item.sectionId = null; });
      writePreview(data); break;
    }
    case 'edit_sidebar_project': data.projects = data.projects.map(p => p.id === args.projectId ? { ...p, name: String(args.name), path: String(args.path) } : p); writePreview(data); break;
    case 'sidebar_project_action': {
      const archives = JSON.parse(localStorage.getItem(archiveKey) ?? '[]') as Session[];
      const selected = data.sessions.filter(s => s.projectId === args.projectId);
      localStorage.setItem(archiveKey, JSON.stringify([...archives, ...selected]));
      data.sessions = data.sessions.filter(s => s.projectId !== args.projectId);
      if (args.action === 'remove') {
        const removed = JSON.parse(localStorage.getItem(removedProjectsKey) ?? '[]') as Project[];
        const p = data.projects.find(p => p.id === args.projectId); if (p) removed.push(p);
        localStorage.setItem(removedProjectsKey, JSON.stringify(removed));
        data.projects = data.projects.filter(p => p.id !== args.projectId);
      }
      writePreview(data); break;
    }
    case 'move_sidebar_session': data.sessions = data.sessions.map(s => s.id === args.sessionId ? { ...s, projectId: String(args.projectId), nativeId: null } : s); writePreview(data); break;
    case 'fork_sidebar_session': {
      const source = data.sessions.find(s => s.id === args.sessionId); if (!source) throw new Error('会话不存在');
      const session = { ...source, id: crypto.randomUUID(), title: `${source.title} · 分叉`, projectId: String(args.projectId ?? source.projectId), nativeId: null, turnId: null, status: 'idle', updatedAt: Date.now() / 1000 };
      data.sessions.unshift(session); writePreview(data); result = session; break;
    }
    case 'list_archived_sessions': {
      const archives = JSON.parse(localStorage.getItem(archiveKey) ?? '[]') as Session[];
      const removed = JSON.parse(localStorage.getItem(removedProjectsKey) ?? '[]') as Project[];
      result = archives.map(session => { const p = [...data.projects, ...removed].find(p => p.id === session.projectId); return { session, projectName: p?.name ?? '', projectPath: p?.path ?? '' }; }); break;
    }
    case 'restore_sidebar_session': {
      const archives = JSON.parse(localStorage.getItem(archiveKey) ?? '[]') as Session[];
      const session = archives.find(s => s.id === args.sessionId); if (session) { data.sessions.unshift(session); const removed = JSON.parse(localStorage.getItem(removedProjectsKey) ?? '[]') as Project[]; const p = removed.find(p => p.id === session.projectId); if (p && !data.projects.some(v => v.id === p.id)) data.projects.push(p); }
      localStorage.setItem(archiveKey, JSON.stringify(archives.filter(s => s.id !== args.sessionId))); writePreview(data); break;
    }
    case 'delete_sidebar_session': {
      data.sessions = data.sessions.filter(s => s.id !== args.sessionId); delete data.sidebar.sessions[String(args.sessionId)];
      const archives = JSON.parse(localStorage.getItem(archiveKey) ?? '[]') as Session[];
      localStorage.setItem(archiveKey, JSON.stringify(archives.filter(s => s.id !== args.sessionId))); writePreview(data); break;
    }
    case 'copy_chat_transcript': result = `# ${data.sessions.find(s => s.id === args.sessionId)?.title ?? '聊天'}\n\n界面预览没有 Agent 消息。`; break;
    case 'runtime_info': result = { running: false, pid: null, shellBytes: 0, agentBytes: 0, requests: [], idleReleaseSeconds: 300, memoryMetric: 'workingSet' }; break;
    case 'release_runtime': break;
    default: throw new Error('当前是界面预览。请运行 npm run desktop，在桌面版中使用本地 Agent。');
  }
  return result as T;
}
export async function subscribe(onEvent: (event: RpcEvent) => void, onLog: (text: string) => void, onStopped: () => void, onWorkspaceUpdated?: () => void) {
  if (!desktop) return () => {};
  const off = await Promise.all([
    listen<RpcEvent>('agent-event', e => onEvent(e.payload)),
    listen<string>('runtime-log', e => onLog(e.payload)),
    listen('runtime-stopped', onStopped),
    listen('workspace-updated', () => onWorkspaceUpdated?.()),
  ]);
  return () => off.forEach(unlisten => unlisten());
}
