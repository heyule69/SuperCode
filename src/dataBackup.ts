import { call, desktop } from './api';
export interface FrontendRestore { token: string; entries: Record<string, string> }
export function backupEntries(storage: Storage): Record<string, string> {
  const entries: Record<string, string> = {};
  for (let index = 0; index < storage.length; index++) {
    const key = storage.key(index);
    if (key?.startsWith('supercode.')) entries[key] = storage.getItem(key) ?? '';
  }
  return entries;
}
export function replaceBackupEntries(storage: Storage, entries: Record<string, string>) {
  if (Object.keys(entries).length > 512 || Object.entries(entries).some(([key, value]) => !key.startsWith('supercode.') || typeof value !== 'string')) throw new Error('备份界面设置无效');
  const previous = backupEntries(storage);
  const replace = (values: Record<string, string>) => {
    for (const key of Object.keys(backupEntries(storage))) storage.removeItem(key);
    for (const [key, value] of Object.entries(values)) storage.setItem(key, value);
  };
  try { replace(entries); }
  catch (error) { replace(previous); throw error; }
}
export async function restoreFrontendSettings() {
  if (!desktop) return;
  const saved = await call<FrontendRestore | null>('restored_frontend');
  if (!saved) return;
  replaceBackupEntries(localStorage, saved.entries);
  await call('acknowledge_frontend_restore', { token: saved.token });
}
