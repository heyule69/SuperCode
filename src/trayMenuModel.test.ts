import { describe, expect, it } from 'vitest';
import { menuKeyIndex, trayStatus } from './trayMenuModel';
describe('tray menu navigation and task state', () => {
  it('shows running and approval state ahead of unread completion', () => {
    expect(trayStatus('running', true)).toBe('running');
    expect(trayStatus('starting', true)).toBe('running');
    expect(trayStatus('waiting', true)).toBe('waiting');
    expect(trayStatus('idle', true)).toBe('unread');
    expect(trayStatus('idle')).toBe('idle');
  });
  it('wraps keyboard focus across the menu', () => {
    expect(menuKeyIndex('ArrowDown', -1, 4)).toBe(0);
    expect(menuKeyIndex('ArrowUp', -1, 4)).toBe(3);
    expect(menuKeyIndex('ArrowDown', 3, 4)).toBe(0);
    expect(menuKeyIndex('ArrowUp', 0, 4)).toBe(3);
    expect(menuKeyIndex('Home', 2, 4)).toBe(0);
    expect(menuKeyIndex('End', 0, 4)).toBe(3);
    expect(menuKeyIndex('ArrowDown', 0, 0)).toBe(-1);
  });
});
