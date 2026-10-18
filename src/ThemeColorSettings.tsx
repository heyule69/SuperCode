import { useEffect, useState } from 'react';
import { RotateCcw } from 'lucide-react';
import type { Preferences } from './preferences';
import { SettingRow, SettingsGroup } from './SettingsPage';
import { effectiveColorTheme, normalizeColor, resolvedThemeColors, type ColorKey } from './themeColors';

function ColorSetting({ title, label, value, customized, change, reset }: { title: string; label: string; value: string; customized: boolean; change: (color: string) => void; reset: () => void }) {
  const [draft, setDraft] = useState(value);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => { setDraft(value); setInvalid(false); }, [value]);
  const commit = () => {
    const color = normalizeColor(draft);
    setInvalid(!color);
    if (color) { setDraft(color); if (color !== value) change(color); }
  };
  return <SettingRow title={title}><div className="theme-color-control">
    <div className="theme-color-field">
      <input type="color" aria-label={`选择${label}`} value={value} onChange={e => { const color = normalizeColor(e.target.value); if (color) { setDraft(color); setInvalid(false); change(color); } }} />
      <input type="text" aria-label={`${label}值`} aria-invalid={invalid} title="输入 #RGB 或 #RRGGBB 颜色" spellCheck={false} autoComplete="off" maxLength={7} value={draft} onChange={e => { setDraft(e.target.value); setInvalid(false); }} onBlur={commit} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.blur(); } if (e.key === 'Escape') { e.stopPropagation(); setDraft(value); setInvalid(false); } }} />
    </div>
    <button className="icon-button" aria-label={`重置${label}`} title={`恢复主题默认${label}`} disabled={!customized} onClick={() => { setDraft(value); setInvalid(false); reset(); }}><RotateCcw size={14}/></button>
  </div></SettingRow>;
}

const fields: { key: ColorKey; title: string; label: string }[] = [{ key: 'accent', title: '强调色', label: '强调色' }, { key: 'background', title: '背景', label: '背景色' }, { key: 'foreground', title: '前景', label: '前景色' }];
export function ThemeColorSettings({ prefs, setPrefs }: { prefs: Preferences; setPrefs: (prefs: Preferences) => void }) {
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches);
  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const update = () => setSystemDark(query.matches);
    query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);
  const theme = effectiveColorTheme(prefs.theme, systemDark);
  const overrides = prefs.themeColors?.[theme] ?? {};
  const colors = resolvedThemeColors(theme, overrides);
  const update = (key: ColorKey, color?: string) => {
    const next = { ...overrides };
    if (color) next[key] = color; else delete next[key];
    const palettes = { ...prefs.themeColors };
    if (Object.keys(next).length) palettes[theme] = next; else delete palettes[theme];
    setPrefs({ ...prefs, themeColors: palettes });
  };
  return <SettingsGroup title="颜色">{fields.map(({ key, title, label }) => <ColorSetting key={key} title={title} label={label} value={colors[key]} customized={!!overrides[key]} change={color => update(key, color)} reset={() => update(key)} />)}</SettingsGroup>;
}
