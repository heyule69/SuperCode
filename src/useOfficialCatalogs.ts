import { useCallback, useEffect, useRef, useState } from 'react';
import { call, desktop } from './api';
import { visibleProviderIds } from './providerOrder';
import type { AgentProfile, ModelCatalog } from './types';

const empty: Record<string, ModelCatalog> = {};

// Browse other verified accounts without switching the chat's active connection.
export function useOfficialCatalogs(enabled: boolean, agent: string, currentId: string, profiles: AgentProfile[], loggedInAgents: string[]) {
  const currentProfile = profiles.find(profile => profile.agent === agent && profile.id === currentId);
  const canonicalCurrent = agent === 'codex' && (currentId === '@local' || currentProfile?.officialAccount && !currentProfile.accountId) ? '@official' : currentId;
  const ids = JSON.stringify(visibleProviderIds(agent, profiles, loggedInAgents).filter(id => id !== canonicalCurrent
    && (id === '@official' || profiles.some(profile => profile.agent === agent && profile.id === id && profile.officialAccount))));
  const key = `${agent}:${ids}`;
  const [result, setResult] = useState({ key: '', catalogs: empty, loading: false, warning: '' });
  const [revision, setRevision] = useState(0);
  const refreshed = useRef(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    const connections: string[] = JSON.parse(ids);
    if (!enabled || !desktop || !connections.length) return;
    let disposed = false;
    const forceRefresh = revision !== refreshed.current;
    refreshed.current = revision;
    setResult({ key, catalogs: empty, loading: true, warning: '' });
    void Promise.allSettled(connections.map(connectionId => call<ModelCatalog>('list_models', { agent, connectionId, forceRefresh }))).then(results => {
      if (disposed) return;
      const catalogs: Record<string, ModelCatalog> = {}, warnings: string[] = [];
      results.forEach((result, index) => {
        if (result.status === 'fulfilled') {
          catalogs[connections[index]] = result.value;
          if (result.value.warning) warnings.push(result.value.warning);
        } else warnings.push(`无法读取官方模型：${String(result.reason)}`);
      });
      setResult({ key, catalogs, loading: false, warning: warnings.join('\n') });
    });
    return () => { disposed = true; };
  }, [enabled, agent, ids, key, revision]);
  return { catalogs: result.key === key ? result.catalogs : empty, loading: enabled && result.key === key && result.loading,
    warning: result.key === key ? result.warning : '', refresh };
}
