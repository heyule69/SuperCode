import type { PermissionMode } from './types';

// Keep the persisted/native mode IDs. Changing the presentation must not widen an existing mode.
export const permissionNames: Record<PermissionMode, string> = {
  read: '只读', strict: '请求批准', ask: '请求批准', auto: '帮我批准', edit: '自动接受编辑', deny: '不询问', full: '完全访问权限',
};
export function permissionOrder(agent: string): PermissionMode[] {
  if (agent === 'codex') return ['ask', 'auto', 'full'];
  if (agent === 'claude') return ['read', 'ask', 'edit', 'auto', 'deny', 'full'];
  if (agent === 'opencode') return ['read', 'ask', 'full'];
  return ['read', 'strict', 'ask', 'full'];
}
export function visiblePermission(mode: PermissionMode, agent: string): PermissionMode {
  return mode === 'strict' && agent !== 'pi' ? 'ask' : mode;
}
export function permissionName(mode: PermissionMode, agent: string, short = false) {
  if (mode === 'full' && short) return '完全访问';
  if (agent === 'claude' && mode === 'read') return '计划模式（只读）';
  if (agent === 'claude' && mode === 'auto') return '自动模式';
  if (agent === 'pi' && mode === 'ask') return '读取自动批准';
  if (agent === 'pi' && mode === 'strict') return '逐项批准';
  return permissionNames[mode];
}
export function isPermissionMode(value: unknown): value is PermissionMode { return typeof value === 'string' && Object.hasOwn(permissionNames, value); }
export function compatiblePermission(value: unknown, agent: string): PermissionMode {
  if (!isPermissionMode(value)) return 'ask';
  // Preserve read-only and legacy manual modes without silently increasing access.
  if (value === 'read' || value === 'strict' || permissionOrder(agent).includes(value)) return value;
  return 'ask';
}
export function loadPermissionChoices(agent: string, defaultMode: unknown): Record<string, PermissionMode> {
  let choices: Record<string, PermissionMode> = {};
  const stored = localStorage.getItem('supercode.permissions.v2');
  try { const raw = JSON.parse(stored ?? '{}');
    if (raw && typeof raw === 'object' && !Array.isArray(raw)) choices = Object.fromEntries(Object.entries(raw).filter(([id, mode]) => ['codex', 'claude', 'opencode', 'pi'].includes(id) && isPermissionMode(mode)).map(([id, mode]) => [id, compatiblePermission(mode, id)]));
  } catch { /* Restore only a valid legacy choice. */ }
  choices[agent] ??= compatiblePermission((stored === null ? localStorage.getItem('supercode.permission') : null) ?? defaultMode, agent);
  return choices;
}

export function permissionDescription(mode: PermissionMode, agent: string) {
  if (mode === 'read') return '分析和读取文件，不修改工作区';
  if (mode === 'full') return '可访问互联网和电脑上的文件，跳过工具审批';
  if (mode === 'edit') return '自动接受文件编辑，运行命令等操作仍按需确认';
  if (mode === 'deny') return '需要审批的操作直接拒绝，不弹出批准请求';
  if (mode === 'auto') return agent === 'codex' ? '自动审查操作风险，需要你决定时请求批准' : '由 Claude 自动评估工具风险并决定是否批准';
  if (mode === 'strict') return agent === 'pi' ? '每次工具调用都先请求你的批准' : '编辑外部文件和使用互联网时请求批准';
  if (agent === 'codex') return '编辑外部文件和使用互联网时请求批准';
  if (agent === 'pi') return '读取自动执行，修改和工具操作请求批准';
  if (agent === 'opencode') return '工具操作请求批准，沿用 OpenCode 审批规则';
  return '使用 Claude 原生手动审批规则，按需请求批准';
}

export function permissionDetails(agent: string) {
  if (agent === 'codex') return '请求批准与帮我批准都保留工作区沙箱；前者交给你确认额外权限，后者使用 Codex 原生自动审查。自动审查拒绝时保留拒绝结果，不自动改为完全访问。';
  if (agent === 'pi') return '逐项批准会确认每次工具调用；读取自动批准会直接执行读取工具，修改和其他工具调用仍向你询问。';
  if (agent === 'opencode') return 'OpenCode 提供读取限定、工具请求批准和工具允许执行三种规则；SuperCode 只传递你明确选择的本次允许或拒绝。';
  return 'Claude 使用原生 plan、manual、acceptEdits、auto、dontAsk 和 bypassPermissions 模式。自动模式由 Claude 评估风险；不询问会拒绝需要审批的操作。能力受本机 Claude 版本和平台策略约束，失败不会切换为完全访问。';
}
