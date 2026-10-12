import { diffLines } from './diff';
import type { Message } from './types';

export interface ActivityFile { path: string; kind?: string; added?: number; removed?: number; diff?: string; truncated?: boolean }
const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
const count = (value: unknown) => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
export function activityFiles(message: Message): ActivityFile[] {
  const data = message.data ?? {};
  const changes = Array.isArray(data.changes) ? data.changes.map(object) : [];
  const summaries = Array.isArray(data.changeSummary) ? data.changeSummary.map(object) : [];
  const rows = summaries.length ? summaries : changes;
  return rows.filter(row => typeof row.path === 'string' && row.path).map(row => {
    const native = changes.find(change => change.path === row.path) ?? row;
    const patch = typeof native.diff === 'string' ? native.diff : undefined;
    const truncated = row.diffTruncated === true || native.diffTruncated === true || /… (?:输出|详情|已省略)/.test(patch ?? '');
    const lines = patch && !truncated && /^@@ /m.test(patch) ? diffLines(patch, Infinity) : undefined;
    const kind = typeof row.kind === 'string' ? row.kind : typeof object(row.kind).type === 'string' ? String(object(row.kind).type) : undefined;
    return { path:String(row.path), kind, diff:patch, truncated,
      added:count(row.added) ?? (lines ? lines.filter(line => line.kind === 'added').length : undefined),
      removed:count(row.removed) ?? (lines ? lines.filter(line => line.kind === 'removed').length : undefined) };
  });
}
export function activityCommand(message: Message): string {
  const data = message.data ?? {};
  return message.kind === 'commandExecution' ? String(data.command ?? message.text.split('\n')[0] ?? '')
    : message.kind === 'claudeToolCall' && data.tool === 'Bash' ? String(object(data.arguments).command ?? '') : '';
}
export function activityOutput(message: Message): string {
  const data = message.data ?? {};
  if (message.kind === 'fileChange') return typeof data.operationOutput === 'string' ? data.operationOutput : typeof data.output === 'string' ? data.output : '';
  if (message.kind !== 'commandExecution') return message.text;
  const command = activityCommand(message);
  return message.text.startsWith(command + '\n') ? message.text.slice(command.length + 1) : message.text === command ? '' : message.text;
}
export function fileAction(kind: string | undefined) {
  return ['add','create','added'].includes(kind ?? '') ? '已创建' : ['delete','remove','deleted'].includes(kind ?? '') ? '已删除' : '已编辑';
}
export function activityDiff(files: ActivityFile[]): string {
  return files.filter(file => !!file.diff).map(file => /^diff --git /m.test(file.diff!) ? file.diff!
    : `diff --git a/${file.path} b/${file.path}\n${file.diff}`).join('\n');
}
