import { call } from './api';

export const PROVIDER_REQUIRED = '请先添加模型供应商，再发送消息。';

export function isProviderRequired(error: unknown): boolean {
  return (error instanceof Error ? error.message : String(error)) === PROVIDER_REQUIRED;
}

export async function requireChatConnection(agent: string, connectionId: string, sessionId?: string): Promise<void> {
  await call('check_chat_connection', { agent, connectionId, sessionId: sessionId || null });
}
