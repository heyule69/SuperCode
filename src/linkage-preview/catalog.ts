import type { LinkageCatalog, LinkageConfiguration } from '../LinkageSetup';

// Isolated design fixtures. These catalogs never read accounts or call agents.
export const catalogs: LinkageCatalog[] = [
  { agent: { id: 'codex', name: 'Codex', installed: true, connected: true, path: null }, connections: [
    { id: 'codex-official', name: 'ChatGPT 官方账号', models: [{ id: 'gpt-codex', name: 'GPT · 编程模型' }, { id: 'gpt-standard', name: 'GPT · 通用模型' }] },
    { id: 'codex-api', name: '自定义 API', models: [{ id: 'gpt-api', name: 'GPT · API 模型' }] },
  ] },
  { agent: { id: 'claude', name: 'Claude Code', installed: true, connected: true, path: null }, connections: [
    { id: 'claude-official', name: 'Claude 官方账号', models: [{ id: 'sonnet', name: 'Claude Sonnet' }, { id: 'opus', name: 'Claude Opus' }] },
    { id: 'claude-api', name: '自定义 API', models: [{ id: 'claude-api', name: 'Claude · API 模型' }] },
  ] },
  { agent: { id: 'opencode', name: 'OpenCode', installed: true, connected: true, path: null }, connections: [
    { id: 'opencode-api', name: '项目 API 连接', models: [{ id: 'glm', name: 'GLM · 编程模型' }, { id: 'claude', name: 'Claude Sonnet' }, { id: 'gpt', name: 'GPT · 编程模型' }] },
  ] },
  { agent: { id: 'pi', name: 'Pi', installed: true, connected: true, path: null }, connections: [
    { id: 'pi-api', name: '项目 API 连接', models: [{ id: 'minimax', name: 'MiniMax · 编程模型' }, { id: 'claude', name: 'Claude Sonnet' }] },
  ] },
  { agent: { id: 'gemini', name: 'Gemini CLI', installed: true, connected: true, path: null }, connections: [
    { id: 'gemini-official', name: 'Google 官方账号', models: [{ id: 'pro', name: 'Gemini Pro' }, { id: 'flash', name: 'Gemini Flash' }] },
    { id: 'gemini-api', name: 'Gemini API', models: [{ id: 'gemini-api', name: 'Gemini · API 模型' }] },
  ] },
];

export const initialConfiguration: LinkageConfiguration = {
  mode: 'single',
  controller: { agentId: 'claude', connectionId: 'claude-official', modelId: 'sonnet' },
  executor: { agentId: 'opencode', connectionId: 'opencode-api', modelId: 'glm' },
};

export function selectionName(choice: LinkageConfiguration['controller']) {
  const catalog = catalogs.find(item => item.agent.id === choice.agentId);
  const connection = catalog?.connections.find(item => item.id === choice.connectionId);
  return { agent: catalog?.agent.name ?? choice.agentId, model: connection?.models.find(item => item.id === choice.modelId)?.name ?? choice.modelId, connection: connection?.name ?? choice.connectionId };
}
