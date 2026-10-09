import type { Skill } from './skills';
export interface SlashCommand { name: string; detail: string; agent?: string; arguments?: string; skill?: Skill }
export const slashCommands: SlashCommand[] = [
  { name: '/new', detail: '新建会话，保留原来的历史' },
  { name: '/clear', detail: '开始空白会话，原会话仍可查看' },
  { name: '/model', detail: '打开模型与供应商设置' },
  { name: '/permissions', detail: '选择当前任务的审批规则' },
  { name: '/status', detail: '查看 Agent 运行状态' },
  { name: '/stop', detail: '停止当前任务' },
  { name: '/goal', detail: '设置并开始 Codex 原生目标，或 pause / resume / clear', agent: 'codex', arguments: '目标内容' },
  { name: '/goal-status', detail: '查看目标、状态与 Token 用量', agent: 'codex' },
  { name: '/compact', detail: '压缩当前对话上下文' },
  { name: '/context', detail: '查看 Claude 原生上下文用量', agent: 'claude' },
  { name: '/usage', detail: '查看 Claude 原生用量信息', agent: 'claude' },
  { name: '/help', detail: '查看此 Agent 的可用命令' },
];
export function commandMatches(input: string, agent: string, native: string[] = [], skills: Skill[] = []) {
  if (!/^\/[\p{L}\p{N}_:.-]*$/iu.test(input)) return [];
  const commands = slashCommands.filter(c => !c.agent || c.agent === agent);
  for (const skill of skills) {
    if (skill.error) continue;
    const name = skill.name.trim().toLowerCase().replace(/[^\p{L}\p{N}_:.-]+/gu, '-');
    if (!name || /^[.-]+$/.test(name)) continue;
    const namespace = skill.namespace?.replace(/[^\p{L}\p{N}_-]+/gu, '-');
    const base = `/${namespace ? `${namespace}:` : ''}${name}`;
    let command = base;
    if (commands.some(c => c.name === command)) command = `/skill:${name}`;
    let suffix = 2;
    const unique = command;
    while (commands.some(c => c.name === command)) command = `${unique}:${suffix++}`;
    commands.push({ name: command, detail: skill.description || '使用此技能', arguments: '任务内容', skill });
  }
  if (['claude', 'opencode', 'pi'].includes(agent)) for (const name of native) {
    const command = `/${name}`;
    if (/^\/[\w:-]+$/.test(command) && !commands.some(c => c.name === command)) commands.push({ name: command, detail: `本机 ${({ claude: 'Claude', opencode: 'OpenCode', pi: 'Pi' } as Record<string,string>)[agent]} 提供的命令` });
  }
  return commands.filter(c => c.name.startsWith(input.toLowerCase()));
}
export function parseCommand(input: string) {
  const match = input.trim().match(/^(\/[\p{L}\p{N}_:.-]+)(?:\s+([\s\S]*))?$/iu);
  return match ? { command: match[1].toLowerCase(), argument: match[2]?.trim() ?? '' } : null;
}
