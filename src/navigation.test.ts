import { describe, expect, it } from 'vitest';
import { emptyNavigation, travel, visit } from './navigation';
const chat = { page: 'chat' as const, projectId: 'project', sessionId: 'chat' };
const settings = { page: 'settings' as const, tab: 'appearance' };
describe('desktop navigation', () => {
  it('settings categories share one destination and returning home keeps the chat', () => {
    let state = visit(visit(emptyNavigation, chat), settings);
    state = visit(state, { page: 'settings', tab: 'usage' });
    state = visit(state, { page: 'settings', tab: 'general' });
    expect(state.entries).toHaveLength(2);
    state = visit(state, chat);
    expect(state.index).toBe(0);
    expect(state.entries[state.index]).toEqual(chat);
    expect(travel(state, 1).entries[1]).toEqual({ page: 'settings', tab: 'general' });
  });
  it('returns to the same chat and can revisit settings', () => {
    const history = visit(visit(emptyNavigation, chat), settings);
    const back = travel(history, -1);
    expect(back.entries[back.index]).toEqual(chat);
    expect(travel(back, 1)).toEqual(history);
    expect(visit(back, chat)).toBe(back);
  });
  it('discards forward history after a new navigation and bounds retained entries', () => {
    const history = visit(visit(emptyNavigation, chat), settings);
    const next = visit(travel(history, -1), { ...chat, sessionId: 'other' });
    expect(next.entries).toEqual([chat, { ...chat, sessionId: 'other' }]);
    expect(travel(next, 1)).toBe(next);
    let bounded = emptyNavigation;
    for (let i = 0; i < 60; i++) bounded = visit(bounded, { ...chat, sessionId: String(i) });
    expect(bounded.entries).toHaveLength(40);
    expect(bounded.index).toBe(39);
    expect(travel(emptyNavigation, -1)).toBe(emptyNavigation);
  });
});
