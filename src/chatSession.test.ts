import { beforeEach, expect, it, vi } from 'vitest';
import { call } from './api';
import { createChatSession } from './chatSession';
import type { Session } from './types';
vi.mock('./api', () => ({ call: vi.fn() }));
beforeEach(() => vi.resetAllMocks());
it('creates the first sent chat with the draft model and provider in one request', async () => {
  const saved = { id: 'sent', connectionId: 'kimi', model: 'k3' } as Session;
  vi.mocked(call).mockResolvedValue(saved);
  expect(await createChatSession({ projectId: 'p', agent: 'claude', model: 'k3', connectionId: 'kimi' })).toBe(saved);
  expect(call).toHaveBeenCalledExactlyOnceWith('create_session', { projectId: 'p', agent: 'claude', model: 'k3', connectionId: 'kimi' });
});
it('leaves default model resolution to the agent when no model was selected', async () => {
  await createChatSession({ projectId: 'p', agent: 'codex', model: '', connectionId: '@official' });
  expect(call).toHaveBeenCalledWith('create_session', { projectId: 'p', agent: 'codex', model: null, connectionId: '@official' });
});
