import { call } from './api';
import type { Theme } from './preferences';

// Serialize theme updates so an older async setTheme cannot overwrite a newer
// selection. Each subscription cancels its queued work when React cleans it up.
let pending = Promise.resolve();

export function watchWindowTheme(theme: Theme, onError: (error: unknown) => void) {
  let disposed = false;
  let revision = 0;
  const query = window.matchMedia('(prefers-color-scheme: dark)');
  const update = () => {
    const current = ++revision;
    const effective = theme === 'system' ? query.matches ? 'dark' : 'light' : theme;
    const root = document.documentElement;
    root.dataset.theme = effective;
    const css = getComputedStyle(root);
    const colors = {
      background: css.getPropertyValue('--window-bg').trim() || css.getPropertyValue('--bg').trim(),
      canvas: css.getPropertyValue('--bg').trim(),
      text: css.getPropertyValue('--text').trim(),
      border: css.getPropertyValue('--line').trim(),
    };
    pending = pending.then(async () => {
      if (disposed || current !== revision) return;
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      if (disposed || current !== revision) return;
      await getCurrentWindow().setTheme(theme === 'system' ? null : ['dark', 'graphite'].includes(effective) ? 'dark' : 'light');
      if (disposed || current !== revision) return;
      await call<boolean>('set_window_colors', { colors });
    }).catch(error => { if (!disposed && current === revision) onError(error); });
  };
  update();
  if (theme === 'system') query.addEventListener('change', update);
  return () => { disposed = true; query.removeEventListener('change', update); };
}
