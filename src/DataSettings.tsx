import { useEffect, useState } from 'react';
import { open, save } from '@tauri-apps/plugin-dialog';
import { call, desktop } from './api';
import { backupEntries } from './dataBackup';
import { SettingRow, SettingsGroup } from './SettingsPage';

interface StorageSummary { categories: { id: string; label: string; bytes: number; reclaimableBytes: number; files: number }[]; pendingRestore: boolean }
const size = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;
export default function DataSettings({ busy }: { busy: boolean }) {
  const [summary, setSummary] = useState<StorageSummary | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [restorePath, setRestorePath] = useState('');
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    void call<StorageSummary>('storage_summary').then(value => { if (!disposed) setSummary(value); }).catch(value => { if (!disposed) setError(String(value)); });
    return () => { disposed = true; };
  }, [revision]);
  async function act(job: () => Promise<void>) {
    if (working) return;
    setWorking(true); setError(''); setNotice('');
    try { await job(); setRevision(value => value + 1); }
    catch (value) { setError(String(value)); }
    finally { setWorking(false); }
  }
  async function exportBackup() {
    const path = await save({ defaultPath: `SuperCode-${new Date().toISOString().replace(/[:.]/g, '-')}.supercode-backup`, filters: [{ name: 'SuperCode 备份', extensions: ['supercode-backup'] }] });
    if (path) await act(async () => { await call('export_data_backup', { path, entries: backupEntries(localStorage) }); setNotice('备份已保存。'); });
  }
  async function chooseRestore() {
    const path = await open({ multiple: false, filters: [{ name: 'SuperCode 备份', extensions: ['supercode-backup'] }] });
    if (typeof path === 'string') setRestorePath(path);
  }
  const locked = busy || working || !desktop;
  return <SettingsGroup title="数据与空间">
    <SettingRow title="备份聊天与设置" description="包含聊天、设置和草稿。凭据仅可在原电脑的 Windows 用户下恢复；项目文件、附件原文件与 Agent 程序请单独保管。"><button className="quiet-button" disabled={working || !desktop} onClick={() => void exportBackup().catch(value => setError(String(value)))}>保存备份</button></SettingRow>
    <SettingRow title="恢复备份" description={summary?.pendingRestore ? '已有待恢复备份，退出并重新打开应用后生效。' : '恢复前自动备份当前数据库；重新打开应用后生效。'}><button className="quiet-button" disabled={locked || summary?.pendingRestore} onClick={() => void chooseRestore().catch(value => setError(String(value)))}>选择备份</button>{summary?.pendingRestore ? <button className="quiet-button" disabled={locked} onClick={() => void call('request_app_exit').catch(value => setError(String(value)))}>退出应用</button> : null}</SettingRow>
    {restorePath ? <div className="setting-row" role="group" aria-label="确认恢复备份"><div className="setting-description"><strong>确认恢复 {restorePath.split(/[\\/]/).at(-1)}</strong><p>当前聊天和设置将被备份内容替换。请退出后重新打开应用。</p></div><div className="setting-control"><button className="quiet-button" disabled={working} onClick={() => setRestorePath('')}>取消</button><button className="primary-button" disabled={locked} onClick={() => void act(async () => { await call('prepare_data_restore', { path: restorePath }); setRestorePath(''); setNotice('备份已验证。退出并重新打开应用即可恢复。'); })}>准备恢复</button></div></div> : null}
    {summary?.categories?.map(category => <SettingRow key={category.id} title={category.label} description={`${category.files} 个文件${category.reclaimableBytes ? ` · 可清理 ${size(category.reclaimableBytes)}` : ''}`}><span>{size(category.bytes)}</span></SettingRow>)}
    <SettingRow title="清理旧安装" description="保留当前与可回滚的 Agent 版本，聊天、附件和项目文件继续保留。"><button className="quiet-button" disabled={locked || !summary?.categories?.some(category => category.reclaimableBytes > 0)} onClick={() => void act(async () => { const result = await call<{ removedBytes: number; removedFiles: number }>('clean_retired_storage'); setNotice(`已清理 ${result.removedFiles} 个文件，释放 ${size(result.removedBytes)}。`); })}>清理旧安装</button></SettingRow>
    {error ? <p className="cc-error" role="alert">{error}</p> : null}
    {notice ? <p className="settings-footnote" role="status">{notice}</p> : null}
  </SettingsGroup>;
}
