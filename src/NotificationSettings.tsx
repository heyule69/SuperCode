import { useCallback, useEffect, useRef, useState } from 'react';
import { Bell, CheckCheck, CircleAlert, RefreshCw, ShieldCheck } from 'lucide-react';
import { call, desktop } from './api';
import type { Preferences } from './preferences';
import type { NotificationStatus } from './notifications';
import { SettingRow, SettingsGroup, SettingsSwitch } from './SettingsPage';
import './notifications.css';

const types = [
  { key: 'notifyComplete', title: '任务完成', description: '任务结束后，提醒你查看结果。', icon: CheckCheck },
  { key: 'notifyError', title: '运行失败', description: '任务出错时，提醒你查看原因。', icon: CircleAlert },
  { key: 'notifyApproval', title: '等待确认', description: '需要审批或回答问题时，提醒你处理。', icon: ShieldCheck },
] as const;

export default function NotificationSettings({ prefs, setPrefs }: { prefs: Preferences; setPrefs: (p: Preferences) => void }) {
  const [status, setStatus] = useState<NotificationStatus>();
  const [checking, setChecking] = useState(false);
  const request = useRef(0);
  const alive = useRef(true);
  const refresh = useCallback(async () => {
    if (!desktop) return;
    const revision = ++request.current; setChecking(true);
    try { const result = await call<NotificationStatus>('notification_status'); if (alive.current && revision === request.current) setStatus(result); }
    catch { if (alive.current && revision === request.current) setStatus({ state: 'unavailable', message: '暂时无法检查系统通知状态' }); }
    finally { if (alive.current && revision === request.current) setChecking(false); }
  }, []);
  useEffect(() => {
    alive.current = true; void refresh(); window.addEventListener('focus', refresh);
    return () => { alive.current = false; request.current++; window.removeEventListener('focus', refresh); };
  }, [refresh]);
  const update = <K extends keyof Preferences>(key: K, value: Preferences[K]) => setPrefs({ ...prefs, [key]: value });
  return <div className="notification-settings">
    <div className="settings-page-heading"><div><h2>通知</h2><p className="notification-subtitle">在需要你关注时提醒你。</p></div></div>
    <div className="notification-system-card">
      <span className="notification-system-icon"><Bell size={20} /></span>
      <div className="notification-system-copy"><strong>系统通知</strong><span role="status">{!desktop ? '在桌面版中检查系统通知' : status?.message ?? '正在检查系统通知状态…'}</span></div>
      <span className={`notification-status-badge ${status?.state === 'enabled' ? 'available' : ''}`}>{!desktop ? '桌面版可用' : status?.state === 'enabled' ? '可用' : status?.state === 'unavailable' || !status ? '待检查' : '已关闭'}</span>
      <button className="icon-button" title="刷新系统通知状态" aria-label="刷新系统通知状态" disabled={!desktop || checking} onClick={() => void refresh()}><RefreshCw size={14} className={checking ? 'spin' : ''} /></button>
    </div>
    <div className="notification-options">
      <SettingsGroup title="提醒方式">
        <SettingRow title="开启通知" description="接收任务完成、失败和等待确认的系统提醒。"><SettingsSwitch label="开启通知" checked={prefs.notificationsEnabled} change={v => update('notificationsEnabled', v)} /></SettingRow>
        <SettingRow title="何时提醒" description={prefs.notificationMode === 'unseen' ? '正在查看的对话保持安静，其他任务仍会提醒。' : prefs.notificationMode === 'background' ? '应用最小化或切换到其他应用时提醒。' : '查看当前对话时也会收到系统提醒。'}>
          <select aria-label="何时提醒" disabled={!prefs.notificationsEnabled} value={prefs.notificationMode} onChange={e => update('notificationMode', e.target.value as Preferences['notificationMode'])}><option value="unseen">未查看的任务</option><option value="background">仅应用在后台</option><option value="always">始终提醒</option></select>
        </SettingRow>
      </SettingsGroup>
      <SettingsGroup title="通知类型">{types.map(type => <div className="notification-type-row" key={type.key}><type.icon size={17} /><SettingRow title={type.title} description={type.description}><SettingsSwitch label={type.title} checked={prefs[type.key]} disabled={!prefs.notificationsEnabled} change={v => update(type.key, v)} /></SettingRow></div>)}</SettingsGroup>
      <SettingsGroup title="声音与隐私">
        <SettingRow title="通知提示音" description="使用系统提示音，跟随系统音量和勿扰设置。"><SettingsSwitch label="通知提示音" checked={prefs.sound} disabled={!prefs.notificationsEnabled} change={v => update('sound', v)} /></SettingRow>
        <SettingRow title="显示项目与任务名称" description="关闭时只显示通知类型，适合共享屏幕。"><SettingsSwitch label="显示项目与任务名称" checked={prefs.notificationDetails} disabled={!prefs.notificationsEnabled} change={v => update('notificationDetails', v)} /></SettingRow>
      </SettingsGroup>
    </div>
  </div>;
}
