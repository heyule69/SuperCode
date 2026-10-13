import { expect, it } from 'vitest';
import { canBindMainThread, requestBelongsToSession } from './sessionEvents';
import type { RpcEvent } from './types';

it.each(['pi','opencode'])('accepts %s authoritative startup rebinding but requires an exact owner', agent => {
  const session = { id:'chat',agent,nativeId:'boot' };
  expect(canBindMainThread(session,{method:'thread/started',params:{appSessionId:'chat',thread:{id:'actual'}}},'actual')).toBe(true);
  expect(canBindMainThread(session,{method:'turn/started',params:{appSessionId:'chat',threadId:'actual'}},'actual')).toBe(true);
  expect(canBindMainThread(session,{method:'thread/started',params:{appSessionId:'other',thread:{id:'actual'}}},'actual')).toBe(false);
  expect(canBindMainThread(session,{method:'thread/started',params:{thread:{id:'actual'}}},'actual')).toBe(false);
});
it('does not replace a bound Codex primary native thread with its child process event', () => {
  const session = { id:'chat',agent:'codex',nativeId:'parent' };
  expect(canBindMainThread(session,{method:'thread/started',params:{appSessionId:'chat',thread:{id:'child',source:{subAgent:{}}}}},'child')).toBe(false);
  expect(canBindMainThread(session,{method:'turn/started',params:{appSessionId:'chat',threadId:'child'}},'child')).toBe(false);
});
it('prefers exact app ownership to a boot or reused native alias for approvals', () => {
  const request: RpcEvent = {id:'request',method:'approval',params:{appSessionId:'chat',threadId:'boot'}};
  expect(requestBelongsToSession(request,'chat',['actual'])).toBe(true);
  expect(requestBelongsToSession({...request,params:{appSessionId:'other',threadId:'actual'}},'chat',['actual'],true)).toBe(false);
  expect(requestBelongsToSession({...request,params:{threadId:'actual'}},'chat',['actual'])).toBe(true);
});
