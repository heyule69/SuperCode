import type { Provider } from './types';

export const protocolLabels: Record<string, string> = { anthropic: 'Anthropic Messages', responses: 'OpenAI Responses', chat: 'OpenAI Chat Completions' };
export const agentProtocols: Record<string, string[]> = {
  claude: ['anthropic'], codex: ['responses'], opencode: ['anthropic', 'responses', 'chat'], pi: ['anthropic', 'responses', 'chat'],
};
export function providersForAgent(providers: Provider[], agent: string): Provider[] {
  const protocols = agentProtocols[agent] ?? [];
  return providers.map(provider => ({ ...provider, presets: provider.presets.filter(preset => protocols.includes(preset.protocol)) })).filter(provider => provider.presets.length > 0);
}
export function officialProvider(agent: string) {
  return agent === 'claude' ? { id: 'anthropic', name: 'Claude 官方登录' } : agent === 'codex' ? { id: 'openai', name: 'ChatGPT 官方登录' } : null;
}
