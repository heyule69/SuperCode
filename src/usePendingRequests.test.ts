import { expect, it } from 'vitest';
import { PendingRequestState } from './usePendingRequests';
import type { RpcEvent } from './types';

const request = (id: string, turnId = 'turn'): RpcEvent => ({ id, method: 'item/commandExecution/requestApproval', params: { threadId: 'thread', turnId } });
it('does not resurrect an approval resolved while an initial snapshot is loading', () => {
  const state = new PendingRequestState(), snapshot = state.begin();
  state.event(request('old'));
  state.event({ method: 'serverRequest/resolved', params: { requestId: 'old' } });
  state.apply(snapshot, [request('old')]);
  expect(state.values()).toEqual([]);
});
it('preserves a new approval received after a reconnect snapshot began', () => {
  const state = new PendingRequestState(), snapshot = state.begin();
  state.event(request('new'));
  state.apply(snapshot, []);
  expect(state.values()).toEqual([request('new')]);
});
it('keeps explicit user replies and turn completions resolved despite old snapshots', () => {
  const state = new PendingRequestState(); state.event(request('reply')); state.event(request('finished'));
  const snapshot = state.begin(); state.resolve('reply'); state.clearTurn('turn');
  state.apply(snapshot, [request('reply'), request('finished')]);
  expect(state.values()).toEqual([]);
});
it('ignores an older overlapping recovery and accepts an authoritative newer snapshot', () => {
  const state = new PendingRequestState(), old = state.begin(), newer = state.begin();
  expect(state.apply(newer, [request('latest')])).toBe(true);
  expect(state.apply(old, [request('stale')])).toBe(false);
  expect(state.values()).toEqual([request('latest')]);
});
it('refuses a snapshot older than the bounded event journal instead of losing resolutions', () => {
  const state = new PendingRequestState(), snapshot = state.begin();
  for (let index = 0; index < 2001; index++) state.resolve(`resolved:${index}`);
  expect(state.apply(snapshot, [request('resolved:0')])).toBe(false);
  expect(state.values()).toEqual([]);
});
