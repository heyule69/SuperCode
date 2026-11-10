import { useCallback, useEffect, useState } from 'react';
import { call, desktop } from './api';
import type { AgentProfile, ModelCatalog } from './types';

interface Catalogs {
  key: string;
  catalogs: Record<string, ModelCatalog>;
  loading: boolean;
  warning: string;
}
const emptyCatalogs: Record<string, ModelCatalog> = {};

// The current connection uses its parent's catalog loader. Other Magpie groups
// need their own read: filtering the menu must not switch the chat's connection.
export function useMagpieCatalogs(enabled: boolean, agent: string, currentId: string, profiles: AgentProfile[]) {
  const connectionIds = JSON.stringify(profiles.filter(profile => profile.agent === agent && profile.id !== currentId
    && (profile.providerId === 'magpie' || profile.modelSource?.providerId === 'magpie')).map(profile => profile.id));
  const key = `${agent}:${connectionIds}`;
  const [result, setResult] = useState<Catalogs>({ key: '', catalogs: {}, loading: false, warning: '' });
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);

  useEffect(() => {
    const ids: string[] = JSON.parse(connectionIds);
    if (!enabled || !desktop || !ids.length) return;
    let disposed = false;
    setResult(previous => ({ key, catalogs: previous.key === key ? previous.catalogs : {}, loading: true, warning: '' }));
    void Promise.allSettled(ids.map(connectionId => call<ModelCatalog>('list_models', { agent, connectionId, forceRefresh: true }))).then(results => {
      if (disposed) return;
      const catalogs: Record<string, ModelCatalog> = {};
      const warnings: string[] = [];
      results.forEach((result, index) => {
        if (result.status === 'fulfilled') {
          catalogs[ids[index]] = result.value;
          const warning = result.value.warning || result.value.notice;
          if (warning) warnings.push(warning);
        } else warnings.push(`无法刷新 Magpie 模型：${result.reason instanceof Error ? result.reason.message : String(result.reason)}`);
      });
      setResult(previous => ({ key, catalogs: { ...(previous.key === key ? previous.catalogs : {}), ...catalogs }, loading: false, warning: warnings.join('\n') }));
    });
    return () => { disposed = true; };
  }, [enabled, agent, connectionIds, key, revision]);

  return { catalogs: result.key === key ? result.catalogs : emptyCatalogs, loading: enabled && result.key === key && result.loading,
    warning: result.key === key ? result.warning : '', refresh };
}
