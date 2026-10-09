import type { Attachment } from './ComposerMenus';
import type { Bootstrap } from './types';

export function restoreWorkspace(data: Pick<Bootstrap, 'projects' | 'sessions'>, saved: { sessionId: string; projectId: string; fresh?: boolean }) {
  if (saved.fresh) return { project: undefined, session: undefined };
  const session = data.sessions.find(s => s.id === saved.sessionId);
  if (session && !session.projectId) return { project: undefined, session };
  const project = data.projects.find(p => p.id === session?.projectId) ?? data.projects.find(p => p.id === saved.projectId);
  return { project, session: project ? session : undefined };
}

export interface DraftModel { connectionId: string; model: string; effort?: string }
export interface Draft { text: string; attachments: Attachment[]; selection?: DraftModel }
export function decodeDrafts(raw: string | null): Map<string, Draft> {
  try {
    const rows: unknown = JSON.parse(raw ?? '[]');
    if (!Array.isArray(rows)) return new Map();
    return new Map(rows.slice(-20).flatMap(row => {
      if (!Array.isArray(row) || row.length !== 2 || typeof row[0] !== 'string' || typeof row[1]?.text !== 'string') return [];
      const attachments = Array.isArray(row[1].attachments) ? row[1].attachments.filter((a: Attachment) => a && ['file', 'image', 'directory', 'skill'].includes(a.kind) && typeof a.path === 'string' && typeof a.name === 'string').slice(0, 12) : [];
      const value = row[1].selection;
      const selection = value && typeof value.connectionId === 'string' && typeof value.model === 'string' ? { connectionId: value.connectionId.slice(0, 256), model: value.model.slice(0, 256), ...(typeof value.effort === 'string' ? { effort: value.effort.slice(0, 64) } : {}) } : undefined;
      return [[row[0], { text: row[1].text.slice(0, 32000), attachments, ...(selection ? { selection } : {}) }] as [string, Draft]];
    }));
  } catch { return new Map(); }
}
export function encodeDrafts(texts: Map<string, string>, attachments: Map<string, Attachment[]>, selections = new Map<string, DraftModel>()) {
  const keys = [...new Set([...texts.keys(), ...attachments.keys(), ...selections.keys()])];
  return JSON.stringify(keys.map(key => [key, { text: (texts.get(key) ?? '').slice(0, 32000), attachments: (attachments.get(key) ?? []).slice(0, 12), ...(selections.has(key) ? { selection: selections.get(key) } : {}) }] as [string, Draft]).filter(([, draft]) => draft.text || draft.attachments.length || draft.selection).slice(-20));
}

export function commandIndex(key: string, current: number, count: number) {
  if (!count) return 0;
  return key === 'ArrowUp' ? (current - 1 + count) % count : (current + 1) % count;
}
