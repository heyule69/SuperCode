import { expect, it } from 'vitest';
import { inUseProviderConnections } from './providerActivity';
import type { Session } from './types';

const session = (agent: string, connectionId: string | null, status: string): Session => ({ id: `${agent}:${connectionId}:${status}`, agent, connectionId, status, projectId: '', title: '', model: null, nativeId: null, turnId: null, updatedAt: 1 });
it('locks every active connection, including background starts and approvals, but releases finished chats', () => {
  const rows = ['starting','running','waiting','idle','failed','interrupted'].map(status => session('claude', status, status));
  rows.push(session('claude', 'running', 'running'), session('codex', 'other', 'running'));
  expect(inUseProviderConnections(rows, [], [])).toEqual({ claude: ['starting','running','waiting'], codex: ['other'] });
});
it('keeps native account IDs scoped to their agent and covers Codex login aliases', () => {
  expect(inUseProviderConnections([session('codex', '@local', 'running'), session('claude', '@official', 'idle')], [], [])).toEqual({ codex: ['@local','@official'] });
  expect(inUseProviderConnections([session('codex', 'native-profile', 'waiting')], [{ id:'native-profile',agent:'codex',name:'Native',model:null,current:false,hasCredential:false,officialAccount:true }], [])).toEqual({ codex: ['native-profile','@official'] });
});
it('resolves old unpinned chats and protects pending routes before session creation', () => {
  const profiles = [{ id:'default',agent:'claude',name:'Default',model:null,current:true,hasCredential:true }];
  expect(inUseProviderConnections([session('claude', null, 'starting'), session('codex', null, 'waiting')], profiles, ['codex'], [{agent:'claude',id:'previous-default'}])).toEqual({ claude:['default','previous-default'], codex:['@official'] });
});
it('keeps a connection locked for a live child after the parent chat completes', () => {
  expect(inUseProviderConnections([session('codex','parent-route','idle')], [], [], [{agent:'claude',id:'side-route'}], {codex:['parent-route']})).toEqual({codex:['parent-route'],claude:['side-route']});
  expect(inUseProviderConnections([session('codex','parent-route','idle')], [], [], [], {})).toEqual({});
});
