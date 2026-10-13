import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { Attachment } from './attachments';
import type { Draft, DraftModel } from './workspaceState';
import { loadWorkspaceDrafts, readWorkspaceDraft, workspaceDraftEventKey, writeWorkspaceDraft } from './workspaceDrafts';

const persistenceWarning = '草稿暂未保存到磁盘，请先复制草稿并检查可用空间。当前窗口仍保留未保存内容。';

export function useWorkspaceDrafts() {
  const [saved] = useState(loadWorkspaceDrafts);
  const drafts = useRef(new Map([...saved].map(([key, draft]) => [key, draft.text])));
  const attachmentDrafts = useRef(new Map([...saved].map(([key, draft]) => [key, draft.attachments])));
  const draftModels = useRef(new Map<string, DraftModel>([...saved].flatMap(([key, draft]) => draft.selection ? [[key, draft.selection] as [string, DraftModel]] : [])));
  const draftKey = useRef('');
  const unsaved = useRef(new Map<string, Draft | undefined>());
  const [persistenceError, setPersistenceError] = useState('');
  const remember = useCallback((key: string, draft?: Draft) => {
    drafts.current.delete(key); attachmentDrafts.current.delete(key); draftModels.current.delete(key);
    if (draft) {
      drafts.current.set(key, draft.text); attachmentDrafts.current.set(key, draft.attachments);
      if (draft.selection) draftModels.current.set(key, draft.selection);
    }
    while (drafts.current.size > 20) {
      const oldest = [...drafts.current.keys()].find(id => id !== key && !unsaved.current.has(id));
      if (oldest === undefined) break;
      drafts.current.delete(oldest); attachmentDrafts.current.delete(oldest); draftModels.current.delete(oldest);
    }
  }, []);
  const read = useCallback((key: string) => { const draft = unsaved.current.has(key) ? unsaved.current.get(key) : readWorkspaceDraft(key); remember(key, draft); return draft; }, [remember]);
  const save = useCallback((key: string, text: string, attachments: Attachment[], selection = draftModels.current.get(key)) => {
    if (!key) return;
    const draft = text || attachments.length || selection ? { text: text.slice(0, 32000), attachments: attachments.slice(0, 12), ...(selection ? { selection } : {}) } : undefined;
    remember(key, draft);
    try { writeWorkspaceDraft(key, draft); unsaved.current.delete(key); } catch { unsaved.current.set(key, draft); }
    setPersistenceError(unsaved.current.size ? persistenceWarning : '');
  }, [remember]);
  const remove = useCallback((key: string) => {
    remember(key);
    try { writeWorkspaceDraft(key, undefined); unsaved.current.delete(key); } catch { unsaved.current.set(key, undefined); }
    setPersistenceError(unsaved.current.size ? persistenceWarning : '');
  }, [remember]);
  const retryPersistence = useCallback(() => {
    for (const [key, draft] of unsaved.current) {
      try { writeWorkspaceDraft(key, draft); unsaved.current.delete(key); } catch { /* Keep every failed row for a later retry. */ }
    }
    setPersistenceError(unsaved.current.size ? persistenceWarning : '');
  }, []);
  useEffect(() => {
    const changed = (event: StorageEvent) => {
      const key = workspaceDraftEventKey(event.key);
      if (key && key !== draftKey.current) read(key);
    };
    window.addEventListener('storage', changed);
    return () => window.removeEventListener('storage', changed);
  }, [read]);
  return { drafts, attachmentDrafts, draftModels, draftKey, read, save, remove, persistenceError, retryPersistence };
}

export function useDraftPersistence(store: ReturnType<typeof useWorkspaceDrafts>, value: { ready: boolean; key: string; text: string; attachments: Attachment[]; selection?: DraftModel }, current: { text: RefObject<string>; attachments: RefObject<Attachment[]> }) {
  const { ready, key, text, attachments, selection } = value;
  const save = store.save;
  useEffect(() => {
    if (!ready || !key) return;
    const persist = () => { if (store.draftKey.current === key) save(key, current.text.current, current.attachments.current, selection); };
    const timer = setTimeout(persist, 250);
    window.addEventListener('beforeunload', persist);
    return () => {
      clearTimeout(timer); window.removeEventListener('beforeunload', persist);
      if (current.text.current === text && current.attachments.current === attachments) persist();
    };
  }, [ready, key, text, attachments, selection?.connectionId, selection?.model, selection?.effort, save, store.draftKey, current.text, current.attachments]);
}
