import { call } from './api';
import { validateTranslation } from './replyTranslation';

export type TranslationState = { status: 'pending' | 'ready' | 'error'; text?: string; error?: string };
interface Entry { sessionId: string; source: string; state: TranslationState; listeners: Set<(state: TranslationState) => void>; running: boolean }

// Only visible consumers enqueue work. Serial requests keep native agents and
// quota use bounded; releasing the last consumer drops work that has not started.
export function createTranslationQueue(request = (sessionId: string, text: string) => call<unknown>('translate_reply', { sessionId, text })) {
  const entries = new Map<string, Entry>();
  let running = false;
  const notify = (entry: Entry) => { for (const listener of entry.listeners) listener(entry.state); };
  function trim() {
    let size = [...entries.values()].reduce((n, e) => n + e.source.length + (e.state.text?.length ?? 0), 0);
    for (const [key, entry] of entries) {
      if (entries.size <= 48 && size <= 256 * 1024) break;
      if (entry.running || entry.listeners.size) continue;
      size -= entry.source.length + (entry.state.text?.length ?? 0);
      entries.delete(key);
    }
  }
  function pump() {
    if (running) return;
    const entry = [...entries.values()].find(e => e.state.status === 'pending' && e.listeners.size && !e.running);
    if (!entry) { trim(); return; }
    running = entry.running = true;
    void request(entry.sessionId, entry.source).then(result => {
      entry.state = { status: 'ready', text: validateTranslation(entry.source, result) };
    }).catch(error => { entry.state = { status: 'error', error: String(error).slice(0, 240) }; }).finally(() => {
      entry.running = running = false;
      notify(entry);
      trim();
      pump();
    });
  }
  return {
    watch(sessionId: string, source: string, listener: (state: TranslationState) => void) {
      const key = `${sessionId}\0${source}`;
      let entry = entries.get(key);
      if (!entry) {
        trim();
        const active = [...entries.values()].filter(e => e.listeners.size || e.running);
        if (active.length >= 48 || active.reduce((n, e) => n + e.source.length, 0) + source.length > 256 * 1024) {
          listener({ status: 'error', error: '当前可见翻译较多，请稍后重试' });
          return () => {};
        }
        entry = { sessionId, source, state: { status: 'pending' }, listeners: new Set(), running: false };
        entries.set(key, entry);
      }
      entry.listeners.add(listener);
      listener(entry.state);
      pump();
      return () => {
        entry.listeners.delete(listener);
        if (!entry.listeners.size && !entry.running && entry.state.status === 'pending') entries.delete(key);
        trim();
      };
    },
    retry(sessionId: string, source: string) {
      const entry = entries.get(`${sessionId}\0${source}`);
      if (entry?.state.status === 'error') { entry.state = { status: 'pending' }; notify(entry); pump(); }
    },
  };
}

export const translationQueue = createTranslationQueue();
