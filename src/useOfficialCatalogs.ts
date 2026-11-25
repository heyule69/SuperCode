import { useCallback, useEffect, useRef, useState } from 'react';
import { call, desktop } from './api';
import { visibleProviderIds } from './providerOrder';
import type { AgentProfile, ModelCatalog } from './types';

const empty: Record<string, ModelCatalog> = {};

// Browse other verified accounts without switching the chat's active connection.
export function useOfficialCatalogs(enabled: boolean, agent: string, currentId: string, profiles: AgentProfile[], loggedInAgents: string[], currentCatalog?: ModelCatalog) {
  const currentProfile = profiles.find(profile => profile.agent === agent && profile.id === currentId);
  const canonicalCurrent = agent === 'codex' && (currentId === '@local' || currentProfile?.officialAccount && !currentProfile.accountId) ? '@official' : currentId;
  const accountIds = visibleProviderIds(agent, profiles, loggedInAgents).filter(id =>
    id === '@official' || profiles.some(profile => profile.agent === agent && profile.id === id && profile.officialAccount));
  const ids = JSON.stringify(accountIds.filter(id => id !== canonicalCurrent));
  // A catalog belongs to an account, not to the chat's currently selected route.
  const key = `${agent}:${JSON.stringify([...accountIds].sort().map(id => [id, profiles.find(profile => profile.agent === agent && profile.id === id)?.accountId]))}`;
  const [result, setResult] = useState({ key: '', catalogs: empty, loading: false, warning: '' });
  const [revision, setRevision] = useState(0);
  const refreshed = useRef(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    if (!enabled || !desktop || !currentCatalog || !accountIds.includes(canonicalCurrent)) return;
    setResult(previous => ({ key, catalogs: { ...(previous.key === key ? previous.catalogs : empty), [canonicalCurrent]: currentCatalog },
      loading: previous.key === key && previous.loading, warning: previous.key === key ? previous.warning : '' }));
  }, [enabled, key, canonicalCurrent, currentCatalog]);
  useEffect(() => {
    const connections: string[] = JSON.parse(ids);
    if (!enabled || !desktop) return;
    if (!connections.length) {
      setResult(previous => ({ key, catalogs: previous.key === key ? previous.catalogs : empty, loading: false, warning: '' }));
      return;
    }
    let disposed = false;
    const forceRefresh = revision !== refreshed.current;
    refreshed.current = revision;
    setResult(previous => ({ key, catalogs: previous.key === key ? previous.catalogs : empty, loading: true, warning: '' }));
    void Promise.allSettled(connections.map(connectionId => call<ModelCatalog>('list_models', { agent, connectionId, forceRefresh }))).then(results => {
      if (disposed) return;
      const loaded: Record<string, ModelCatalog> = {}, warnings: string[] = [];
      results.forEach((result, index) => {
        if (result.status === 'fulfilled') {
          loaded[connections[index]] = result.value;
          if (result.value.warning) warnings.push(result.value.warning);
        } else warnings.push(`无法读取官方模型：${String(result.reason)}`);
      });
      setResult(previous => {
        const catalogs = previous.key === key ? { ...previous.catalogs } : {};
        for (const id of connections) delete catalogs[id];
        return { key, catalogs: { ...catalogs, ...loaded }, loading: false, warning: warnings.join('\n') };
      });
    });
    return () => { disposed = true; };
  }, [enabled, agent, ids, key, revision]);
  return { catalogs: result.key === key ? result.catalogs : empty, loading: enabled && ids !== '[]' && (result.key !== key || result.loading),
    warning: result.key === key ? result.warning : '', refresh };
}
