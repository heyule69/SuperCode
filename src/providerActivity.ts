import { isActive, type AgentProfile, type Session } from './types';

export interface ProviderConnection { agent: string; id: string }
export type RunningConnections = Record<string, string[]>;

export function inUseProviderConnections(sessions: Session[], profiles: AgentProfile[], officialAgents: string[], pending: Iterable<ProviderConnection> = [], runningConnections: RunningConnections = {}): RunningConnections {
  const used: RunningConnections = {};
  function add(agent: string, id: string) {
    const ids = used[agent] ??= [];
    if (!ids.includes(id)) ids.push(id);
    const profile = profiles.find(row => row.agent === agent && row.id === id);
    if ((agent === 'codex' && id === '@local' || profile?.officialAccount && !profile.accountId) && !ids.includes('@official')) ids.push('@official');
  }
  for (const [agent, ids] of Object.entries(runningConnections)) for (const id of ids) add(agent, id);
  for (const session of sessions) {
    if (!isActive(session.status)) continue;
    const id = session.connectionId ?? profiles.find(row => row.agent === session.agent && row.current)?.id ?? (officialAgents.includes(session.agent) ? '@official' : '@local');
    add(session.agent, id);
  }
  for (const connection of pending) add(connection.agent, connection.id);
  return used;
}
