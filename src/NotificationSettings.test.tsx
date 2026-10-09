import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import NotificationSettings from './NotificationSettings';
import { defaults } from './preferences';
import { notificationPreferences } from './notifications';

describe('notification privacy and preferences', () => {
  it('keeps the task-name privacy preference in sync with the setting', () => {
    for (const details of [false, true]) {
      const prefs = { ...defaults, notificationDetails: details };
      const html = renderToStaticMarkup(<NotificationSettings prefs={prefs} setPrefs={() => {}} />);
      const control = html.match(/<input[^>]*aria-label="显示项目与任务名称"[^>]*>/)?.[0];
      expect(control).toBeDefined();
      expect(control?.includes('checked=""')).toBe(details);
      expect(notificationPreferences(prefs).details).toBe(details);
    }
  });
  it('disables subordinate controls without clearing saved choices', () => {
    const prefs = { ...defaults, notificationsEnabled: false, sound: true };
    const html = renderToStaticMarkup(<NotificationSettings prefs={prefs} setPrefs={() => {}} />);
    expect(html).toMatch(/aria-label="通知提示音"[^>]*disabled=""[^>]*checked=""/);
    expect(html).toMatch(/aria-label="何时提醒"[^>]*disabled=""/);
    expect(notificationPreferences(prefs)).toMatchObject({ enabled: false, sound: true, complete: true, mode: 'unseen', details: false });
  });
});
