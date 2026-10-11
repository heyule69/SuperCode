import { call } from './api';

export async function openRuntimeLogFolder(): Promise<void> {
  return call<void>('open_runtime_log_folder');
}

export async function exportRuntimeDiagnostics(): Promise<string | null> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const path = await save({ title: '导出诊断日志', defaultPath: 'SuperCode-diagnostics.jsonl',
    filters: [{ name: '诊断日志', extensions: ['jsonl', 'txt'] }] });
  if (!path) return null;
  return call<string>('save_runtime_diagnostics', { path });
}
