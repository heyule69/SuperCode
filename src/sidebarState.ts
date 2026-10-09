import type { Project, Session, SidebarState } from './types';

export const emptySidebar: SidebarState = { projects: {}, sessions: {}, sections: [] };
export interface SidebarGroup { id: string; name: string; projects: Project[]; sessions: Session[] }
export function sidebarGroups(projects: Project[], sessions: Session[], state: SidebarState): SidebarGroup[] {
  const pinned: SidebarGroup = { id: 'pinned', name: '置顶', projects: [], sessions: [] };
  const recent: SidebarGroup = { id: 'recent', name: '最近对话', projects: [], sessions: [] };
  const main: SidebarGroup = { id: 'default', name: '项目', projects: [], sessions: [] };
  const groups = state.sections.map(s => ({ ...s, projects: [] as Project[], sessions: [] as Session[] }));
  const byId = new Map(groups.map(group => [group.id, group]));
  for (const project of projects) {
    const item = state.projects[project.id];
    (item?.pinned ? pinned : byId.get(item?.sectionId ?? '') ?? main).projects.push(project);
  }
  const projectIds = new Set(projects.map(p => p.id));
  for (const session of sessions) {
    if (session.projectId && !projectIds.has(session.projectId)) continue;
    const item = state.sessions[session.id];
    if (item?.pinned) pinned.sessions.push(session);
    else if (item?.sectionId && byId.has(item.sectionId)) byId.get(item.sectionId)!.sessions.push(session);
    else if (!session.projectId) recent.sessions.push(session);
  }
  return [pinned, main, recent, ...groups].filter(group => group.projects.length || group.sessions.length || byId.has(group.id));
}
export function sidebarAreas(projects: Project[], sessions: Session[], state: SidebarState) {
  const groups = sidebarGroups(projects, sessions, state);
  const sections = new Set(state.sections.map(section => section.id));
  return {
    projects: groups.map(group => ({ ...group, sessions: group.sessions.filter(session => !!session.projectId) }))
      .filter(group => group.projects.length || group.sessions.length || sections.has(group.id)),
    direct: groups.map(group => ({ ...group, projects: [], sessions: group.sessions.filter(session => !session.projectId) }))
      .filter(group => group.sessions.length),
  };
}
export function projectSessions(projectId: string, sessions: Session[], state: SidebarState) {
  const sections = new Set(state.sections.map(s => s.id));
  return sessions.filter(s => s.projectId === projectId && !state.sessions[s.id]?.pinned && !sections.has(state.sessions[s.id]?.sectionId ?? ''));
}
export function exportFilename(title: string, format: 'html' | 'md') {
  return `${title.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/, '').slice(0, 80) || '聊天'}.${format}`;
}
