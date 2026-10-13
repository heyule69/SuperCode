import { decodeDrafts, encodeDrafts, type Draft } from './workspaceState';

export const draftPrefix = 'supercode.draft.v2:';
export const draftMigrationKey = 'supercode.drafts.migration.v2';
interface StoredDraft { version: 2; updatedAt: number; draft: Draft | null }
const storageKey = (key: string) => draftPrefix + encodeURIComponent(key);

function readRow(raw: string | null): StoredDraft | undefined {
  try {
    const row = JSON.parse(raw ?? 'null');
    if (row?.version !== 2 || !Number.isFinite(row.updatedAt)) return;
    if (row.draft === null) return row;
    const draft = decodeDrafts(JSON.stringify([['draft', row.draft]])).get('draft');
    if (draft) return { version: 2, updatedAt: row.updatedAt, draft };
  } catch { /* A damaged row does not prevent other drafts from loading. */ }
}

/** Immutable legacy snapshot: migration never writes over a live v2 row. */
function legacyDrafts(storage: Storage) {
  let raw = storage.getItem(draftMigrationKey);
  if (raw === null) {
    raw = storage.getItem('supercode.drafts.v1') ?? '[]';
    try { storage.setItem(draftMigrationKey, raw); } catch { /* Read-only recovery still works. */ }
  }
  return decodeDrafts(raw);
}

export function readWorkspaceDraft(key: string, storage = localStorage): Draft | undefined {
  const row = readRow(storage.getItem(storageKey(key)));
  return row ? row.draft ?? undefined : legacyDrafts(storage).get(key);
}

export function loadWorkspaceDrafts(storage = localStorage): Map<string, Draft> {
  const rows = new Map([...legacyDrafts(storage)].map(([key, draft], index) => [key, { draft, updatedAt: index }]));
  for (let index = 0; index < storage.length; index++) {
    const key = storage.key(index);
    if (!key?.startsWith(draftPrefix)) continue;
    const row = readRow(storage.getItem(key));
    if (!row) continue;
    try {
      const id = decodeURIComponent(key.slice(draftPrefix.length));
      if (row.draft) rows.set(id, { draft: row.draft, updatedAt: row.updatedAt });
      else rows.delete(id);
    } catch { /* Ignore malformed keys. */ }
  }
  return new Map([...rows].sort((a, b) => a[1].updatedAt - b[1].updatedAt).slice(-20).map(([key, row]) => [key, row.draft]));
}

/** Each chat has one atomic write. Other windows' rows are never replayed. */
export function writeWorkspaceDraft(key: string, draft: Draft | undefined, storage = localStorage) {
  const bounded = draft && decodeDrafts(encodeDrafts(new Map([[key, draft.text]]), new Map([[key, draft.attachments]]), draft.selection ? new Map([[key, draft.selection]]) : undefined)).get(key);
  storage.setItem(storageKey(key), JSON.stringify({ version: 2, updatedAt: Date.now(), draft: bounded ?? null } satisfies StoredDraft));
}

export function workspaceDraftEventKey(key: string | null) {
  if (!key?.startsWith(draftPrefix)) return;
  try { return decodeURIComponent(key.slice(draftPrefix.length)); } catch { return; }
}
