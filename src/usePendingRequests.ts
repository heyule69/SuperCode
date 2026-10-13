import { useCallback, useEffect, useRef, useState } from 'react';
import type { RpcEvent } from './types';

type RequestId = string | number;
interface Snapshot { revision: number; generation: number }

/** Snapshots are reconciled with every request/resolve observed after the read began. */
export class PendingRequestState {
  private rows = new Map<RequestId, RpcEvent>();
  private changes = new Map<RequestId, { revision: number; request?: RpcEvent }>();
  private revision = 0;
  private generation = 0;
  private discardedThrough = 0;
  values() { return [...this.rows.values()]; }
  private change(id: RequestId, request?: RpcEvent) {
    if (request) this.rows.set(id, request); else this.rows.delete(id);
    this.changes.delete(id); this.changes.set(id, { revision: ++this.revision, request });
    if (this.changes.size > 2000) {
      const first = this.changes.keys().next().value!;
      this.discardedThrough = this.changes.get(first)!.revision;
      this.changes.delete(first);
    }
  }
  event(event: RpcEvent) {
    if (event.id !== undefined) this.change(event.id, event);
    else if (event.method === 'serverRequest/resolved' && event.params.requestId !== undefined) this.resolve(event.params.requestId);
  }
  resolve(id: RequestId) { this.change(id); }
  clearTurn(turnId: string) { for (const request of this.rows.values()) if (request.params.turnId === turnId) this.resolve(request.id!); }
  begin(): Snapshot { return { revision: this.revision, generation: ++this.generation }; }
  apply(snapshot: Snapshot, requests: RpcEvent[]) {
    if (snapshot.generation !== this.generation || snapshot.revision < this.discardedThrough) return false;
    const rows = new Map(requests.filter(request => request.id !== undefined).map(request => [request.id!, request]));
    for (const [id, change] of this.changes) if (change.revision > snapshot.revision) {
      if (change.request) rows.set(id, change.request); else rows.delete(id);
    }
    this.rows = rows;
    return true;
  }
}

export function usePendingRequests() {
  const [requests, setRequests] = useState<RpcEvent[]>([]);
  const [state] = useState(() => new PendingRequestState());
  const alive = useRef(true);
  const publish = useCallback(() => { if (alive.current) setRequests(state.values()); }, [state]);
  const event = useCallback((value: RpcEvent) => { state.event(value); if (value.id !== undefined || value.method === 'serverRequest/resolved') publish(); }, [state, publish]);
  const resolve = useCallback((id: RequestId) => { state.resolve(id); publish(); }, [state, publish]);
  const clearTurn = useCallback((id: string) => { state.clearTurn(id); publish(); }, [state, publish]);
  const begin = useCallback(() => state.begin(), [state]);
  const apply = useCallback((snapshot: Snapshot, rows: RpcEvent[]) => { if (state.apply(snapshot, rows)) publish(); }, [state, publish]);
  const recover = useCallback(async (load: () => Promise<RpcEvent[]>) => { const snapshot = state.begin(); const rows = await load(); if (alive.current && state.apply(snapshot, rows)) publish(); }, [state, publish]);
  useEffect(() => { alive.current = true; return () => { alive.current = false; state.begin(); }; }, [state]);
  return { requests, event, resolve, clearTurn, begin, apply, recover };
}
