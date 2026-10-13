import type { RpcEvent, Session } from './types';

/** Pi/ACP can replace their provisional boot ID; a Codex child's owner is its process parent. */
export function canBindMainThread(session: Pick<Session, 'id' | 'agent' | 'nativeId'>, event: RpcEvent, native: unknown) {
  if (typeof native !== 'string' || !native || !['thread/started', 'turn/started'].includes(event.method)) return false;
  if (!session.nativeId || session.nativeId === native) return !event.params.thread?.source?.subAgent;
  return ['pi', 'opencode'].includes(session.agent) && event.params.appSessionId === session.id && !event.params.thread?.source?.subAgent;
}

export function requestBelongsToSession(request: RpcEvent, sessionId: string, natives: readonly (string | null | undefined)[], allowUnscoped = false) {
  const owner = request.params.appSessionId;
  if (owner !== undefined && owner !== null) return !!sessionId && owner === sessionId;
  return allowUnscoped && !request.params.threadId || natives.includes(request.params.threadId);
}
