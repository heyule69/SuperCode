export const colorThemes = ['light', 'dark', 'graphite', 'sand'] as const;
export type ColorTheme = typeof colorThemes[number];
export type ColorKey = 'accent' | 'background' | 'foreground';
export type ThemeColors = Partial<Record<ColorKey, string>>;
export type ThemeColorPreferences = Partial<Record<ColorTheme, ThemeColors>>;
export const themeBaseColors: Record<ColorTheme, Record<ColorKey, string>> = {
  light: { accent: '#424A54', background: '#F3F4F5', foreground: '#292B2E' },
  dark: { accent: '#C3C8CE', background: '#181818', foreground: '#E3E3E3' },
  graphite: { accent: '#A4C1EF', background: '#20262F', foreground: '#E1E8F1' },
  sand: { accent: '#88613F', background: '#FCFAF5', foreground: '#39362F' },
};
export function normalizeColor(value: unknown): string | undefined {
  if (typeof value !== 'string' || !/^#(?:[\da-f]{3}|[\da-f]{6})$/i.test(value.trim())) return;
  const hex = value.trim().slice(1);
  return `#${hex.length === 3 ? [...hex].map(c => c + c).join('') : hex}`.toUpperCase();
}
export function normalizeThemeColors(value: unknown): ThemeColorPreferences {
  const result: ThemeColorPreferences = {};
  if (!value || typeof value !== 'object') return result;
  for (const theme of colorThemes) {
    const input = (value as Record<string, unknown>)[theme];
    if (!input || typeof input !== 'object') continue;
    const colors: ThemeColors = {};
    for (const key of ['accent', 'background', 'foreground'] as const) {
      const color = normalizeColor((input as Record<string, unknown>)[key]);
      if (color) colors[key] = color;
    }
    if (Object.keys(colors).length) result[theme] = colors;
  }
  return result;
}
export function effectiveColorTheme(theme: string, systemDark: boolean): ColorTheme {
  return theme === 'system' ? systemDark ? 'dark' : 'light' : colorThemes.includes(theme as ColorTheme) ? theme as ColorTheme : 'light';
}
function rgb(hex: string) { return [1, 3, 5].map(start => parseInt(hex.slice(start, start + 2), 16)); }
function luminance(hex: string) {
  const c = rgb(hex).map(n => { const s = n / 255; return s <= .04045 ? s / 12.92 : ((s + .055) / 1.055) ** 2.4; });
  return c[0] * .2126 + c[1] * .7152 + c[2] * .0722;
}
function readable(color: string, background: string) {
  const a = luminance(color), b = luminance(background);
  return (Math.max(a, b) + .05) / (Math.min(a, b) + .05) >= 4.5 ? color : b > .179 ? '#000000' : '#FFFFFF';
}
function mix(color: string, background: string, amount: number) {
  const base = rgb(background);
  return '#' + rgb(color).map((n, i) => Math.round(n * amount + base[i] * (1 - amount)).toString(16).padStart(2, '0')).join('').toUpperCase();
}
export function resolvedThemeColors(theme: ColorTheme, overrides: ThemeColors = {}): Record<ColorKey, string> {
  const base = themeBaseColors[theme];
  const background = normalizeColor(overrides.background) ?? base.background;
  return { background, foreground: normalizeColor(overrides.foreground) ?? readable(base.foreground, background), accent: normalizeColor(overrides.accent) ?? readable(base.accent, background) };
}
export function themeColorVariables(theme: ColorTheme, overrides: ThemeColors = {}): Record<string, string> {
  if (!Object.values(overrides).some(normalizeColor)) return {};
  const { background, foreground, accent } = resolvedThemeColors(theme, overrides);
  const vars: Record<string, string> = { '--accent': accent, '--accent-bg': mix(accent, background, luminance(background) < .179 ? .2 : .1), '--on-accent': readable('#FFFFFF', accent) };
  if (overrides.accent) vars['--chat-link'] = accent;
  if (overrides.background || overrides.foreground) {
    Object.assign(vars, { '--text': foreground, '--line': mix(foreground, background, .14), '--muted': mix(foreground, background, .6), '--hover': mix(foreground, background, .06) });
  }
  if (overrides.background) {
    Object.assign(vars, { '--bg': background, '--window-bg': background, '--sidebar': mix(foreground, background, .03), '--surface': mix(foreground, background, .02), '--surface2': mix(foreground, background, .06), '--composer-bg': mix(foreground, background, .05), '--chat-user-bg': mix(foreground, background, .08) });
  }
  return vars;
}
const colorProperties = ['--accent', '--accent-bg', '--on-accent', '--chat-link', '--bg', '--window-bg', '--sidebar', '--surface', '--surface2', '--composer-bg', '--chat-user-bg', '--text', '--line', '--muted', '--hover'];
export function applyThemeColors(root: HTMLElement, theme: ColorTheme, preferences: ThemeColorPreferences = {}) {
  const vars = themeColorVariables(theme, preferences[theme]);
  for (const key of colorProperties) {
    if (vars[key]) root.style.setProperty(key, vars[key]); else root.style.removeProperty(key);
  }
}
