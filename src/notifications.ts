import type { Preferences } from './preferences';

export type NotificationKind = 'complete' | 'error' | 'approval';
export interface NotificationContext { sessionId: string; settingsOpen: boolean }
export interface NotificationStatus { state: 'enabled' | 'appBlocked' | 'systemBlocked' | 'policyBlocked' | 'unavailable'; message: string }
export function notificationPreferences(prefs: Preferences) {
  return { enabled: prefs.notificationsEnabled, mode: prefs.notificationMode, complete: prefs.notifyComplete, error: prefs.notifyError, approval: prefs.notifyApproval, sound: prefs.sound, details: prefs.notificationDetails };
}
