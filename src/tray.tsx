import { useCallback, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { TrayMenu } from './TrayMenu';
import type { TraySnapshot } from './trayMenuModel';
import { applyPreferences, loadPreferences } from './preferences';
import { watchWindowTheme } from './windowTheme';
import './themes.css';
import './tray.css';

function TrayApp() {
  const [data, setData] = useState<TraySnapshot>();
  const [error, setError] = useState('');
  const [prefs, setPrefs] = useState(loadPreferences);
  const report = useCallback((value: unknown) => setError(String(value)), []);
  useEffect(() => {
    applyPreferences(prefs);
    return watchWindowTheme(prefs.theme, report);
  }, [prefs, report]);
  useEffect(() => {
    let disposed = false; let revision = 0;
    const subscriptions: (() => void)[] = [];
    const updatePreferences = () => { const next = loadPreferences(); setPrefs(previous => JSON.stringify(previous) === JSON.stringify(next) ? previous : next); };
    const load = async () => {
      const current = ++revision;
      try { const snapshot = await invoke<TraySnapshot>('tray_menu_snapshot'); if (!disposed && current === revision) { setData(snapshot); setError(''); updatePreferences(); } }
      catch (value) { if (!disposed && current === revision) report(value); }
    };
    const storage = (event: StorageEvent) => { if (event.key === 'supercode.preferences') updatePreferences(); };
    window.addEventListener('storage', storage);
    for (const event of ['tray-menu-open', 'tray-menu-updated', 'workspace-updated']) {
      void listen(event, () => { void load(); }).then(off => { if (disposed) off(); else subscriptions.push(off); }).catch(report);
    }
    void load();
    return () => { disposed = true; subscriptions.forEach(off => off()); window.removeEventListener('storage', storage); };
  }, [report]);
  const ready = useCallback((height: number) => { if (data?.token) void invoke('present_tray_menu', { token:data.token, height }).catch(report); }, [data?.token, report]);
  const perform = useCallback(async (action: string, value?: string) => {
    try { await invoke('tray_menu_action', { action, value }); } catch (value) { report(value); }
  }, [report]);
  return <TrayMenu data={data} error={error} ready={ready} perform={perform}/>;
}
createRoot(document.getElementById('root')!).render(<TrayApp/>);
