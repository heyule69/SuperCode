import { call } from './api';
import type { Theme } from './preferences';
import { applyThemeColors, type ThemeColorPreferences } from './themeColors';

// Serialize theme updates so an older async setTheme cannot overwrite a newer
// selection. Each subscription cancels its queued work when React cleans it up.
let pending = Promise.resolve();
let nativeTheme: 'light' | 'dark' | null | undefined;

export function watchWindowTheme(theme: Theme, onError: (error: unknown) => void, colors?: ThemeColorPreferences) {
  let disposed = false;
  let revision = 0;
  const query = window.matchMedia('(prefers-color-scheme: dark)');
  const update = () => {
    const current = ++revision;
    const effective = theme === 'system' ? query.matches ? 'dark' : 'light' : theme;
    const root = document.documentElement;
    root.dataset.theme = effective;
    applyThemeColors(root, effective, colors);
    const css = getComputedStyle(root);
    const windowColors = {
      background: css.getPropertyValue('--window-bg').trim() || css.getPropertyValue('--bg').trim(),
      canvas: css.getPropertyValue('--bg').trim(),
      text: css.getPropertyValue('--text').trim(),
      border: css.getPropertyValue('--line').trim(),
    };
    pending = pending.then(async () => {
      if (disposed || current !== revision) return;
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      if (disposed || current !== revision) return;
      const mode = theme === 'system' ? null : ['dark', 'graphite'].includes(effective) ? 'dark' : 'light';
      // Color-picker changes must not repeatedly reset the native theme/canvas.
      if (nativeTheme !== mode) { await getCurrentWindow().setTheme(mode); nativeTheme = mode; }
      if (disposed || current !== revision) return;
      await call<boolean>('set_window_colors', { colors: windowColors });
    }).catch(error => { if (!disposed && current === revision) onError(error); });
  };
  update();
  if (theme === 'system') query.addEventListener('change', update);
  return () => { disposed = true; query.removeEventListener('change', update); };
}
