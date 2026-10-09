// Local visual QA only. Replays captured real Agent events; no inference or credentials.
import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { DatabaseSync } from 'node:sqlite';
import { join } from 'node:path';
const root = join(process.cwd(), '.supercode');
const folders = await readdir(join(root, 'smoke'));
const candidates = await Promise.all(folders.map(async folder => {
  const path = join(root, 'smoke', folder, 'supercode.db');
  try { return { path, mtime: (await stat(path)).mtimeMs }; } catch { return null; }
}));
candidates.sort((a, b) => (b?.mtime ?? 0) - (a?.mtime ?? 0));
const result = {};
for (const agent of ['claude', 'codex']) {
  const trace = JSON.parse(await readFile(join(root, `activity-${agent}-events.json`), 'utf8'));
  const native = trace.find(e => e.method === 'turn/started')?.params.threadId;
  for (const candidate of candidates) {
    if (!candidate) continue;
    const db = new DatabaseSync(candidate.path, { readOnly: true });
    try {
      const session = db.prepare('SELECT id, agent FROM sessions WHERE native_id=?').get(native);
      if (!session) continue;
      const messages = db.prepare('SELECT seq,id,session_id AS sessionId,role,text,kind,data FROM messages WHERE session_id=? ORDER BY seq').all(session.id).map(m => ({ ...m, data: JSON.parse(m.data) }));
      result[agent] = { sessionId: session.id, trace, messages };
      break;
    } finally { db.close(); }
  }
  if (!result[agent]) throw new Error(`Missing real ${agent} session`);
}
await writeFile(join(root, 'activity-fixtures.json'), JSON.stringify(result, null, 2), 'utf8');
console.log('Prepared Claude and Codex real event replays for local UI verification.');
