import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaults, loadPreferences } from './preferences';

function stored(value: object) {
  vi.stubGlobal('localStorage', { getItem: (key: string) => key === 'supercode.preferences' ? JSON.stringify(value) : null });
}
afterEach(() => vi.unstubAllGlobals());
describe('typography preference migration', () => {
  it('upgrades the previous system defaults while retaining theme and behavior', () => {
    stored({ typographyVersion: 1, uiFont: 'system', uiSize: 14, chatSize: 16, theme: 'dark', animations: false });
    expect(loadPreferences()).toMatchObject({ uiSize: 13, chatSize: 14, theme: 'dark', animations: false, typographyVersion: 2 });
  });
  it('retains custom typography and explicit choices after migration', () => {
    for (const custom of [{ uiFont: 'Microsoft YaHei', uiSize: 13, chatSize: 14 }, { uiFont: 'system', uiSize: 15, chatSize: 18 }, { typographyVersion: 2, uiFont: 'system', uiSize: 14, chatSize: 16 }]) {
      stored(custom);
      expect(loadPreferences()).toMatchObject(custom);
    }
  });
  it('uses the new defaults on a new installation', () => {
    stored({});
    expect(loadPreferences()).toEqual(defaults);
  });
});
describe('notification preferences', () => {
  it('preserves previous notification and sound choices when adding new controls', () => {
    stored({ notifyComplete: false, notifyError: true, notifyApproval: false, sound: true });
    expect(loadPreferences()).toMatchObject({ notifyComplete: false, notifyError: true, notifyApproval: false, sound: true, notificationsEnabled: true, notificationMode: 'unseen', notificationDetails: false });
  });
  it('retains disabled notifications, privacy and timing across restarts', () => {
    stored({ notificationsEnabled: false, notificationMode: 'background', notificationDetails: true });
    expect(loadPreferences()).toMatchObject({ notificationsEnabled: false, notificationMode: 'background', notificationDetails: true });
  });
  it('recovers malformed notification options to valid defaults', () => {
    stored({ notificationsEnabled: 'true', notificationMode: 'later', notificationDetails: 1, notifyComplete: null, sound: 'false' });
    expect(loadPreferences()).toMatchObject({ notificationsEnabled: true, notificationMode: 'unseen', notificationDetails: false, notifyComplete: true, sound: false });
  });
});

it('restores Agent-compatible native default permissions without widening legacy read-only', () => {
  for (const [defaultAgent, defaultPermission, expected] of [['codex', 'edit', 'ask'], ['codex', 'read', 'read'], ['claude', 'edit', 'edit'], ['pi', 'auto', 'ask']]) {
    stored({ defaultAgent, defaultPermission });
    expect(loadPreferences().defaultPermission).toBe(expected);
  }
});
