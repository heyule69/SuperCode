import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { call } from './api';
import type { Followup, FollowupPayload } from './Followups';

interface Edit { row: Followup; token: string; originalDraft: FollowupPayload }
interface Options {
  getDraft: () => FollowupPayload;
  setDraft: (draft: FollowupPayload, sessionId: string) => void;
  reload: () => Promise<void>;
  report: (error: unknown) => void;
  focus: () => void;
}

// The editor lives in the composer. A backend reservation holds the original
// row in place even if the active turn ends while the user is still typing.
export function useFollowupEditor(sessionId: string, options: Options) {
  const edits = useRef(new Map<string, Edit>());
  const operations = useRef(new Set<string>());
  const selected = useRef(sessionId); selected.current = sessionId;
  const callbacks = useRef(options); callbacks.current = options;
  const focusRequested = useRef<string | undefined>(undefined);
  const disposed = useRef(false);
  const [, refresh] = useState(0);
  const update = () => refresh(value => value + 1);
  useEffect(() => {
    disposed.current = false;
    // Release only this composer's reservations when the pane/window closes.
    // Cancellation has no supplier request and is sent to IPC immediately.
    const release = () => {
      for (const edit of edits.current.values()) {
        void call('finish_followup_edit', { id: edit.row.id, token: edit.token, payload: null }).catch(() => {});
      }
      edits.current.clear();
    };
    window.addEventListener('beforeunload', release);
    return () => { disposed.current = true; window.removeEventListener('beforeunload', release); release(); };
  }, []);
  useLayoutEffect(() => {
    if (focusRequested.current === sessionId && !operations.current.has(sessionId)) {
      focusRequested.current = undefined;
      callbacks.current.focus();
    }
  });
  async function begin(row: Followup) {
    const key = row.sessionId;
    if (operations.current.has(key) || edits.current.has(key)) return;
    const originalDraft = structuredClone(callbacks.current.getDraft());
    operations.current.add(key); update();
    try {
      const edit = await call<{ row: Followup; token: string }>('begin_followup_edit', { id: row.id });
      if (disposed.current) {
        await call('finish_followup_edit', { id: edit.row.id, token: edit.token, payload: null });
        return;
      }
      edits.current.set(key, { ...edit, originalDraft });
      callbacks.current.setDraft(edit.row.payload, key);
      await callbacks.current.reload();
      focusRequested.current = key;
    } catch (error) { callbacks.current.report(error); }
    finally { operations.current.delete(key); update(); }
  }
  async function finish(payload: FollowupPayload | null) {
    const key = selected.current, edit = edits.current.get(key);
    if (!edit || operations.current.has(key)) return;
    operations.current.add(key); update();
    try {
      await call('finish_followup_edit', { id: edit.row.id, token: edit.token, payload });
      edits.current.delete(key);
      callbacks.current.setDraft(edit.originalDraft, key);
      await callbacks.current.reload();
      focusRequested.current = key;
    } catch (error) { callbacks.current.report(error); }
    finally { operations.current.delete(key); update(); }
  }
  return { editingId: edits.current.get(sessionId)?.row.id, payloadFor: (id: string) => edits.current.get(id)?.row.payload, busy: operations.current.has(sessionId), begin, save: (payload: FollowupPayload) => finish(payload), cancel: () => finish(null) };
}
