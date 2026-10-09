import type { AgentProfile, ModelSource } from './types';

export type ConnectionOrder = Record<string, string[]>;
export const providerAgents = [{ id: 'claude', name: 'Claude Code' }, { id: 'codex', name: 'Codex' }, { id: 'opencode', name: 'OpenCode' }, { id: 'pi', name: 'Pi' }];

export function connectionIds(agent: string, profiles: AgentProfile[], officialAgents: string[], order?: ConnectionOrder): string[] {
  const saved = order?.[agent];
  const own = profiles.filter(p => p.agent === agent);
  const defaultId = own.find(p => p.current)?.id ?? (officialAgents.includes(agent) ? '@official' : '@local');
  const canonical = (id: string) => agent === 'codex' && (id === '@local' || own.some(p => p.id === id && p.officialAccount && !p.accountId)) ? '@official' : id;
  const nativeIds = ['opencode', 'pi'].includes(agent) ? ['@local'] : ['@official'];
  const available = new Set([...own.map(p => canonical(p.id)), ...nativeIds]);
  if (agent !== 'codex' && (saved?.includes('@local') || !saved && defaultId === '@local')) available.add('@local');
  return [...new Set([...(saved ?? [defaultId]), ...own.map(p => p.id), ...nativeIds].map(canonical))].filter(id => available.has(id));
}

export function nativeConnectionSource(agent: string, id: string): ModelSource {
  return id === '@official'
    ? { providerId: agent === 'claude' ? 'anthropic' : 'openai', providerName: agent === 'claude' ? 'Anthropic' : 'OpenAI', connectionName: agent === 'claude' ? 'Claude 官方账号' : 'ChatGPT 官方账号', mark: agent === 'claude' ? 'A' : 'O' }
    : { providerId: 'custom', providerName: '本机 CLI 配置', connectionName: '本机 CLI 配置', mark: '本' };
}

// The persisted route list also contains legacy CLI fallbacks. They are not
// configured suppliers; only a confirmed native login creates an account row.
export function visibleProviderIds(agent: string, profiles: AgentProfile[], loggedInAgents: string[], order?: ConnectionOrder): string[] {
  const loggedIn = loggedInAgents.includes(agent) && ['codex', 'claude'].includes(agent);
  return connectionIds(agent, profiles, loggedInAgents, order).filter(id => {
    if (id === '@local') return false;
    if (id === '@official') return loggedIn;
    const profile = profiles.find(p => p.agent === agent && p.id === id);
    return !!profile && (!profile.officialAccount || (profile.accountId ? loggedInAgents.includes(id) : loggedIn));
  });
}

export function mergeVisibleOrder(visible: string[], complete: string[]): string[] {
  return [...visible, ...complete.filter(id => !visible.includes(id))];
}

export function moveConnection(ids: string[], id: string, target: string, edge: 'before' | 'after' = 'before'): string[] {
  if (id === target || !ids.includes(id) || !ids.includes(target)) return ids;
  const next = ids.filter(v => v !== id);
  next.splice(next.indexOf(target) + (edge === 'after' ? 1 : 0), 0, id);
  return next;
}
