import { useCallback, useEffect, useState } from 'react';
import { call, desktop } from './api';

// Native account capabilities belong to this exact connection and model.
// A delayed response must never enable a tier on another account or model.
export function useCodexSpeeds(enabled: boolean, connectionId: string, model: string) {
  const key = `${connectionId}:${model}`;
  const [state, setState] = useState<{ key: string; tiers?: string[]; warning?: string }>({ key: '' });
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    if (!enabled || !desktop || !connectionId || !model) return;
    let disposed = false;
    setState({ key });
    void call<{ tiers: string[] }>('get_codex_model_speeds', { connectionId, model }).then(result => {
      if (!disposed) setState({ key, tiers: result.tiers });
    }).catch(error => { if (!disposed) setState({ key, warning: `无法核对速度：${String(error)}` }); });
    return () => { disposed = true; };
  }, [enabled, connectionId, model, key, revision]);
  const current = enabled && state.key === key ? state : undefined;
  return { tiers: current?.tiers, warning: current?.warning, loading: enabled && desktop && !current?.tiers && !current?.warning, refresh };
}
