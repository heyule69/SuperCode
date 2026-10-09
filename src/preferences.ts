import { compatiblePermission } from './permissions';
export type Theme = 'light' | 'dark' | 'graphite' | 'sand' | 'system';
export interface Preferences {
  typographyVersion?: number;
  theme: Theme; uiFont: string; codeFont: string; uiSize: number; chatSize: number;
  animations: boolean; notifyComplete: boolean; notifyError: boolean; notifyApproval: boolean; sound: boolean;
  notificationsEnabled: boolean; notificationMode: 'unseen' | 'background' | 'always'; notificationDetails: boolean;
  enterSend: boolean; defaultAgent: string; defaultPermission: string;
}
export const systemFont = '-apple-system-body, ui-sans-serif, -apple-system, system-ui, "Segoe UI", Helvetica, "Microsoft YaHei", Arial, sans-serif, "Apple Color Emoji", "Segoe UI Emoji"';
export const defaults: Preferences = { typographyVersion: 2, theme: 'light', uiFont: 'system', codeFont: 'Consolas', uiSize: 13, chatSize: 14, animations: true, notifyComplete: true, notifyError: true, notifyApproval: true, sound: false, notificationsEnabled: true, notificationMode: 'unseen', notificationDetails: false, enterSend: true, defaultAgent: 'claude', defaultPermission: 'ask' };
export function loadPreferences(): Preferences {
  try {
    const raw = JSON.parse(localStorage.getItem('supercode.preferences') ?? '{}');
    const prefs = { ...defaults, theme: localStorage.getItem('supercode.theme') as Theme ?? defaults.theme, ...raw };
    // Upgrade the previous system default once; keep customized typography intact.
    if (raw.typographyVersion === 1 && prefs.uiFont === 'system' && raw.uiSize === 14 && raw.chatSize === 16) {
      prefs.uiSize = defaults.uiSize; prefs.chatSize = defaults.chatSize;
    }
    prefs.typographyVersion = defaults.typographyVersion;
    prefs.defaultPermission = compatiblePermission(prefs.defaultPermission, prefs.defaultAgent);
    for (const key of ['notificationsEnabled', 'notificationDetails', 'notifyComplete', 'notifyError', 'notifyApproval', 'sound'] as const) {
      if (typeof prefs[key] !== 'boolean') prefs[key] = defaults[key];
    }
    if (!['unseen', 'background', 'always'].includes(prefs.notificationMode)) prefs.notificationMode = defaults.notificationMode;
    return prefs;
  } catch { return { ...defaults }; }
}
export function applyPreferences(p: Preferences) {
  const root = document.documentElement;
  root.dataset.theme = p.theme === 'system' ? window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light' : p.theme;
  root.dataset.motion = p.animations ? 'on' : 'off';
  root.style.setProperty('--ui-font', p.uiFont === 'system' ? systemFont : `"${p.uiFont}", "Microsoft YaHei", sans-serif`);
  root.style.setProperty('--code-font', `"${p.codeFont}", Consolas, monospace`);
  root.style.setProperty('--ui-size', `${p.uiSize}px`); root.style.setProperty('--chat-size', `${p.chatSize}px`);
  localStorage.setItem('supercode.preferences', JSON.stringify(p)); localStorage.setItem('supercode.theme', p.theme);
}
