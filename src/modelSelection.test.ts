// @vitest-environment happy-dom
import { beforeEach, expect, it } from 'vitest';
import { initialModelSelection, loadModelSelection, saveModelSelection } from './modelSelection';
import type { AgentProfile } from './types';

const profiles: AgentProfile[] = ['claude','codex'].map(agent => ({id:`api:${agent}`,agent,name:agent,model:'default',hasCredential:true,current:true}));
const chosen = {connectionId:'api:claude',model:'opus',effort:'max'};
beforeEach(() => localStorage.clear());

it.each(['fast','ultrafast'])('retains official speed %s per selection and reads older saved selections', serviceTier => {
  const official = {connectionId:'@official',model:'official-model',effort:'ultra',serviceTier};
  saveModelSelection('codex', official);
  expect(loadModelSelection('codex', profiles, ['codex'])).toEqual(official);
  expect(initialModelSelection('codex', profiles, ['codex'], {text:'unfinished',attachments:[],selection:{...official,serviceTier:'default'}})?.serviceTier).toBe('default');
  saveModelSelection('codex', {...official,serviceTier:undefined});
  expect(loadModelSelection('codex', profiles, ['codex'])?.serviceTier).toBeUndefined();
});

it('persists the last explicit selection independently for each Agent', () => {
  saveModelSelection('claude', chosen);
  const codex = {connectionId:'@official',model:'gpt',effort:'high'};
  saveModelSelection('codex', codex);
  expect(loadModelSelection('claude', profiles, ['codex'])).toEqual(chosen);
  expect(loadModelSelection('codex', profiles, ['codex'])).toEqual(codex);
});

it('falls back after a connection is removed or belongs to another Agent', () => {
  saveModelSelection('claude', chosen);
  expect(loadModelSelection('claude', [], [])).toBeUndefined();
  saveModelSelection('codex', chosen);
  expect(loadModelSelection('codex', profiles, [])).toBeUndefined();
  localStorage.setItem('supercode.model-selection.v1:claude', '{broken');
  expect(loadModelSelection('claude', profiles, [])).toBeUndefined();
});

it('keeps an unfinished draft while fresh windows and empty drafts prefer the last choice', () => {
  saveModelSelection('claude', chosen);
  const old = {...chosen,model:'older',effort:'low'};
  const draft = {text:'unfinished',attachments:[],selection:old};
  expect(initialModelSelection('claude', profiles, [], draft)).toEqual(old);
  expect(initialModelSelection('claude', profiles, [], draft, true)).toEqual(chosen);
  expect(initialModelSelection('claude', profiles, [], {...draft,text:''})).toEqual(chosen);
  expect(initialModelSelection('claude', [], [], draft, true)).toBeUndefined();
});
