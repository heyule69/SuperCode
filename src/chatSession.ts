import { call } from './api';
import type { Session } from './types';

// Only the send path materializes a draft; its provider is bound in the same insert.
export function createChatSession({ projectId, agent, model, connectionId }: { projectId: string; agent: string; model: string; connectionId: string }): Promise<Session> {
  return call<Session>('create_session', { projectId, agent, model: model || null, connectionId });
}
