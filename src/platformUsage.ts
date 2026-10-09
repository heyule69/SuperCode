import { useCallback, useEffect, useRef, useState } from 'react';
import { call, desktop } from './api';
import type { ModelSource } from './types';

export interface AllowanceWindow { label: string; remainingPercent: number; resetAt?: string | null }
export interface PlatformUsage {
  connectionId: string; agent: string; providerId?: string; providerName: string; connectionName: string; planName?: string | null;
  windows: AllowanceWindow[]; balances: { label: string; amount: number; currency: string }[]; queriedAt: number;
}
export interface QuotaConnection { connectionId: string; agent: string; connectionName: string; source: ModelSource }
type CachedUsage = { revision: string; at: number; value: PlatformUsage | null };
type UsageRequest = { revision: string; force: boolean; promise: Promise<PlatformUsage | null> };
export class UsageQueries {
  private cache = new Map<string, CachedUsage>();
  private pending = new Map<string, UsageRequest>();
  constructor(private read: (agent: string, connectionId: string, refresh: boolean) => Promise<PlatformUsage | null>, private now = Date.now) {}
  peek(agent: string, connectionId: string, revision = '') {
    const key = JSON.stringify([agent, connectionId]);
    const saved = this.cache.get(key);
    if (!saved || saved.revision !== revision) return undefined;
    if (this.now() - saved.at >= 300_000) { this.cache.delete(key); return undefined; }
    this.cache.delete(key); this.cache.set(key, saved);
    return saved.value;
  }
  query(agent: string, connectionId: string, force = false, revision = ''): Promise<PlatformUsage | null> {
    const key = JSON.stringify([agent, connectionId]);
    const running = this.pending.get(key);
    if (running?.revision === revision) {
      return force && !running.force ? running.promise.then(value => {
        const currentRevision = this.pending.get(key)?.revision ?? this.cache.get(key)?.revision;
        return currentRevision !== undefined && currentRevision !== revision ? value : this.query(agent, connectionId, true, revision);
      }) : running.promise;
    }
    const saved = this.cache.get(key);
    if (!force && saved?.revision === revision && this.now() - saved.at < (saved.value ? 60_000 : 20_000)) {
      this.peek(agent, connectionId, revision);
      return Promise.resolve(saved.value);
    }
    const request: Promise<PlatformUsage | null> = Promise.resolve().then(() => this.read(agent, connectionId, force)).catch(() => null).then(value => {
      if (this.pending.get(key)?.promise === request) {
        this.cache.delete(key); this.cache.set(key, { revision, at: this.now(), value });
        if (this.cache.size > 64) this.cache.delete(this.cache.keys().next().value!);
      }
      return value;
    }).finally(() => { if (this.pending.get(key)?.promise === request) this.pending.delete(key); });
    this.pending.set(key, { revision, force, promise: request });
    return request;
  }
}
const queries = new UsageQueries((agent, connectionId, refresh) => call<PlatformUsage | null>('get_platform_usage', { agent, connectionId, refresh }));
export function queryPlatformUsage(agent: string, connectionId: string, refresh = false, revision = '') {
  return queries.query(agent, connectionId, refresh, revision);
}
export function allowanceSummary(value: PlatformUsage) {
  if (value.windows.length) return `剩余 ${Math.floor(Math.min(...value.windows.map(window => window.remainingPercent)))}%`;
  const balance = value.balances[0];
  return balance ? `${balance.label} ${balance.currency === 'CNY' ? '¥' : balance.currency === 'USD' ? '$' : ''}${balance.amount.toFixed(2)}` : '';
}
export function resetLabel(resetAt?: string | null) {
  if (!resetAt) return '';
  const date = /^\d+$/.test(resetAt) ? new Date(Number(resetAt) * 1000) : new Date(resetAt);
  return Number.isFinite(date.getTime()) ? `${date.toLocaleString(undefined, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })} 重置` : '';
}
export function usePlatformUsage(agent: string, connectionId: string, revision: string) {
  const key = JSON.stringify([agent, connectionId, revision]);
  const serial = useRef(0);
  const [state, setState] = useState<{ key: string; value: PlatformUsage | null; loading: boolean }>(() => ({ key, value: queries.peek(agent, connectionId, revision) ?? null, loading: desktop }));
  const refresh = useCallback(async (force = false) => {
    if (!desktop) return;
    const request = ++serial.current;
    setState(old => ({ key, value: old.key === key ? old.value : queries.peek(agent, connectionId, revision) ?? null, loading: true }));
    const value = await queryPlatformUsage(agent, connectionId, force, revision);
    if (serial.current === request) setState({ key, value, loading: false });
  }, [agent, connectionId, key, revision]);
  useEffect(() => {
    if (!desktop) return;
    const cached = queries.peek(agent, connectionId, revision);
    setState({ key, value: cached ?? null, loading: true });
    // Rapid selection changes should not start a request for each intermediate connection.
    const first = window.setTimeout(() => void refresh(), cached === undefined ? 120 : 0);
    const timer = window.setInterval(() => { if (document.visibilityState === 'visible') void refresh(); }, 120_000);
    return () => { ++serial.current; window.clearTimeout(first); window.clearInterval(timer); };
  }, [agent, connectionId, key, revision, refresh]);
  return { value: state.key === key ? state.value : queries.peek(agent, connectionId, revision) ?? null, loading: state.key === key ? state.loading : desktop, refresh };
}
