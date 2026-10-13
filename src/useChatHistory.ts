import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { call } from './api';
import { mergeEvent, reconcileMessages } from './events';
import type { Message, RpcEvent } from './types';

export function useChatHistory(current: RefObject<{ sessionId: string }>, pendingUsers: RefObject<Set<string>>, pendingMessages: RefObject<Map<string, Message>>, scrollArea: RefObject<HTMLElement | null>, prependScroll: RefObject<{ height: number; top: number } | null>) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const revision = useRef(0);
  const queue = useRef<RpcEvent[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const alive = useRef(true);
  const loadMessages = useCallback(async (id: string, before?: number) => {
    if (current.current.sessionId !== id) return;
    const request = ++revision.current;
    setMessagesLoading(true);
    try {
      const items = await call<Message[]>('list_messages', { sessionId: id, before: before ?? null });
      if (!alive.current || current.current.sessionId !== id || request !== revision.current) return;
      if (before && scrollArea.current) prependScroll.current = { height: scrollArea.current.scrollHeight, top: scrollArea.current.scrollTop };
      const pending = new Set(pendingUsers.current);
      for (const message of items) if (message.role === 'user' && typeof message.data?.clientMessageId === 'string') { pendingUsers.current.delete(message.data.clientMessageId); pendingMessages.current.delete(message.data.clientMessageId); }
      const optimistic = [...pendingMessages.current.values()].filter(message => message.sessionId === id);
      // The native ActivityBuffer can already include a queued delta. Consume it
      // into live state before reconciliation, rather than append it after the snapshot.
      const events = queue.current.splice(0);
      if (timer.current) clearTimeout(timer.current); timer.current = undefined;
      setMessages(old => {
        const live = events.reduce((messages, event) => mergeEvent(messages, event, id), old);
        const ids = new Set(live.map(message => message.id));
        return before ? [...items.filter(message => !ids.has(message.id)), ...live] : reconcileMessages([...live, ...optimistic.filter(message => !ids.has(message.id))], items, pending);
      });
      setHasMore(items.length === 100);
    } finally { if (alive.current && current.current.sessionId === id && request === revision.current) setMessagesLoading(false); }
  }, [current, pendingUsers, pendingMessages, scrollArea, prependScroll]);
  const flush = useCallback(() => {
    if (timer.current) clearTimeout(timer.current); timer.current = undefined;
    const events = queue.current.splice(0), id = current.current.sessionId;
    if (events.length && alive.current) setMessages(old => events.reduce((items, event) => mergeEvent(items, event, id), old));
  }, [current]);
  const enqueue = useCallback((event: RpcEvent) => { queue.current.push(event); if (!timer.current) timer.current = setTimeout(flush, 50); }, [flush]);
  const discard = useCallback(() => { if (timer.current) clearTimeout(timer.current); timer.current = undefined; queue.current = []; }, []);
  useEffect(() => { alive.current = true; return () => { alive.current = false; revision.current++; discard(); }; }, [discard]);
  return { messages, setMessages, messagesLoading, setMessagesLoading, hasMore, setHasMore, loadMessages, flush, enqueue, discard };
}
