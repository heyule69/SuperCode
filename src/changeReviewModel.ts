import { diffFiles, diffLines } from './diff';
import type { Changes } from './types';

export interface ReviewFile { path: string; status: string; added: number; removed: number; diff: string }
export function reviewFiles(changes: Changes): ReviewFile[] {
  const patches = new Map<string, ReturnType<typeof diffFiles>[number]>();
  for (const patch of diffFiles(changes.diff)) {
    const previous = patches.get(patch.path);
    // A file can appear in both the working tree and staged diff.
    patches.set(patch.path, previous ? { path: patch.path, added: previous.added + patch.added, removed: previous.removed + patch.removed, diff: previous.diff + patch.diff } : patch);
  }
  return changes.files.map(file => ({ added: 0, removed: 0, diff: '', ...patches.get(file.path), ...file }));
}
export type ReviewLine = ReturnType<typeof diffLines>[number];
export interface ReviewHunk { oldStart: number; nextStart: number; lines: ReviewLine[]; context: number }
export function reviewHunks(text: string, limit = 5000) {
  const hunks: ReviewHunk[] = [];
  let current: ReviewHunk | undefined;
  for (const line of diffLines(text, limit)) {
    if (line.kind === 'hunk') {
      const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line.text)!;
      current = { oldStart: Number(match[1]), nextStart: Number(match[2]), lines: [], context: 0 };
      hunks.push(current);
    } else if (current && (line.old !== null || line.next !== null)) {
      current.lines.push({ ...line, text: line.text.slice(1) });
      if (line.kind === 'neutral') current.context++;
    } else if (line.text.startsWith('diff --git')) current = undefined;
  }
  return { hunks, truncated: text.split('\n').length > limit, binary: /(?:Binary files .* differ|GIT binary patch)/.test(text), noFinalNewline: text.includes('\\ No newline at end of file') };
}
