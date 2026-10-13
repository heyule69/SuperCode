import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { call, subscribe } from './api';
import { isDisplayEvent, mergeEvent } from './events';
import { usePendingRequests } from './usePendingRequests';
import { canBindMainThread, requestBelongsToSession } from './sessionEvents';
import type { Bootstrap, Message, RuntimeInfo, Session } from './types';

const terminal = (message: Message) => ['completed', 'failed', 'interrupted', 'declined'].includes(String(message.data?.status));

/** Prefer observed updates without replaying a delta already persisted in the snapshot. */
export function reconcileStreamSnapshot(live: Message[], saved: Message[], changed: ReadonlySet<string>) {
  const byId = new Map(live.map(message => [message.id, message]));
  const ids = new Set(saved.map(message => message.id));
  const result = saved.map(message => {
    const previous = byId.get(message.id);
    if (!previous || !changed.has(message.id)) return message;
    const observed = terminal(previous) && !terminal(message) || terminal(previous) === terminal(message) && previous.text.length >= message.text.length;
    return observed ? { ...previous, seq: message.seq, historyRun: message.historyRun ?? previous.historyRun } : message;
  });
  return [...result, ...live.filter(message => changed.has(message.id) && !ids.has(message.id))];
}

export function useSideChatRecovery(current: RefObject<Session | undefined>, setSession: (session: Session) => void, report: (error: unknown) => void) {
  const [messages, setMessages] = useState<Message[]>([]);
  const live = useRef<Message[]>([]);
  const pending = usePendingRequests();
  const { event: requestEvent, resolve, clearTurn, begin, apply } = pending;
  const callbacks = useRef({ setSession, report }); callbacks.current = { setSession, report };
  const alive = useRef(true), generation = useRef(0), eventRevision = useRef(0);
  const changed = useRef(new Map<string, number>());
  const discardedThrough = useRef(0);
  const publishTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const publish = useCallback(() => { if (publishTimer.current) clearTimeout(publishTimer.current); publishTimer.current = undefined; if (alive.current) setMessages(live.current); }, []);
  const refresh = useCallback(async () => {
    const session = current.current; if (!session) return;
    const request = ++generation.current, revision = eventRevision.current, snapshot = begin();
    const [data, items, info] = await Promise.all([call<Bootstrap>('bootstrap'), call<Message[]>('list_messages', { sessionId: session.id, limit: 120 }), call<RuntimeInfo>('runtime_info')]);
    if (!alive.current || request !== generation.current || current.current?.id !== session.id || revision < discardedThrough.current) return;
    const saved = data.sessions.find(row => row.id === session.id);
    // Turn/request events may have advanced status or native binding while the read was in flight.
    if (saved && revision === eventRevision.current) { current.current = saved; callbacks.current.setSession(saved); }
    const nativeId = current.current?.nativeId ?? saved?.nativeId;
    apply(snapshot, info.requests.filter(event => requestBelongsToSession(event, session.id, [nativeId])));
    const touched = new Set([...changed.current].filter(([, at]) => at > revision).map(([id]) => id));
    live.current = reconcileStreamSnapshot(live.current, items, touched); publish();
    for (const [id, at] of changed.current) if (at <= revision) changed.current.delete(id);
  }, [current, begin, apply, publish]);
  useEffect(() => {
    alive.current = true;
    let disposed = false, off: (() => void) | undefined;
    const schedule = () => {
      if (!current.current || refreshTimer.current) return;
      refreshTimer.current = setTimeout(() => { refreshTimer.current = undefined; void refresh().catch(error => { if (!disposed) callbacks.current.report(error); }); }, 150);
    };
    void subscribe(event => {
      if (disposed) return;
      const session = current.current, native = event.params?.threadId ?? event.params?.thread?.id;
      if (!session) return;
      const owner = event.params?.appSessionId;
      if (owner !== undefined && owner !== null && owner !== session.id) return;
      const bind = owner === session.id && canBindMainThread(session, event, native);
      const ownedRequest = owner === session.id && (event.id !== undefined || event.method === 'serverRequest/resolved');
      if (native !== session.nativeId && !bind && !ownedRequest) return;
      requestEvent(event); eventRevision.current++;
      if (event.method === 'error') callbacks.current.report(event.params.error?.message ?? 'Agent 运行失败');
      if (bind || event.id !== undefined || event.method === 'serverRequest/resolved' || event.method === 'turn/started' || event.method === 'turn/completed') {
        const next = { ...session, ...(bind ? { nativeId: native } : {}), ...(event.method === 'turn/started' ? { status: 'running', turnId: event.params.turn.id } : event.method === 'turn/completed' ? { status: event.params.turn.status === 'completed' ? 'idle' : event.params.turn.status, turnId: null } : event.id !== undefined ? { status: 'waiting' } : event.method === 'serverRequest/resolved' && session.status === 'waiting' ? { status: 'running' } : {}) };
        current.current = next; callbacks.current.setSession(next);
        if (event.method === 'turn/completed') clearTurn(event.params.turn.id);
        if (bind || event.method === 'turn/started' || event.method === 'turn/completed') schedule();
      }
      if (isDisplayEvent(event)) {
        const previous = new Map(live.current.map(message => [message.id, message]));
        live.current = mergeEvent(live.current, event, session.id);
        for (const message of live.current) if (previous.get(message.id) !== message) {
          changed.current.delete(message.id); changed.current.set(message.id, eventRevision.current);
        }
        while (changed.current.size > 2000) { const oldest = changed.current.keys().next().value!; discardedThrough.current = changed.current.get(oldest)!; changed.current.delete(oldest); }
        if (!publishTimer.current) publishTimer.current = setTimeout(publish, 50);
      }
    }, () => {}, schedule, schedule).then(cleanup => { if (disposed) cleanup(); else off = cleanup; }).catch(error => { if (!disposed) callbacks.current.report(error); });
    return () => {
      disposed = true; alive.current = false; generation.current++; off?.();
      if (refreshTimer.current) clearTimeout(refreshTimer.current); refreshTimer.current = undefined;
      if (publishTimer.current) clearTimeout(publishTimer.current); publishTimer.current = undefined;
    };
  }, [current, requestEvent, clearTurn, refresh, publish]);
  return { messages, requests: pending.requests, refresh, resolve };
}
