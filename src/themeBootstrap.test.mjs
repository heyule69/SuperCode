import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const bootstrap = readFileSync(new URL('../public/theme-init.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const palettes = readFileSync(new URL('./themes.css', import.meta.url), 'utf8');
function initialize(preferences, saved, dark = false, throws = false) {
  const root = { dataset: {} };
  runInNewContext(bootstrap, { document: { documentElement: root }, window: { matchMedia: () => ({ matches: dark }) },
    localStorage: { getItem: (key) => { if (throws) throw new Error('Storage unavailable'); return key === 'supercode.preferences' ? preferences : saved; } },
  });
  return root.dataset.theme;
}
describe('theme before the first React frame', () => {
  it('restores explicit and legacy palettes before the app loads', () => {
    for (const theme of ['light', 'dark', 'graphite', 'sand']) {
      expect(initialize(JSON.stringify({ theme }), 'light', true)).toBe(theme);
      expect(initialize(null, theme)).toBe(theme);
    }
    expect(html.indexOf('src="/theme-init.js"')).toBeLessThan(html.indexOf('id="root"'));
    expect(html).not.toMatch(/<script[^>]*theme-init[^>]*(?:defer|async|type="module")/);
  });
  it('resolves system colors and remains usable with invalid or unavailable storage', () => {
    expect(initialize('{"theme":"system"}', null, true)).toBe('dark');
    expect(initialize('{"theme":"system"}', null, false)).toBe('light');
    expect(initialize('{"theme":"invalid"}', null)).toBe('light');
    expect(initialize('broken JSON', 'dark')).toBe('light');
    expect(initialize(null, null, false, true)).toBe('light');
  });
  it('uses the same early backing colors as the full stylesheet', () => {
    for (const [theme, color] of [['dark', '#181818'], ['graphite', '#20262f'], ['sand', '#fcfaf5']]) {
      expect(html).toContain(`html[data-theme=${theme}] { background:${color};`);
      const palette = palettes.split(`:root[data-theme=${theme}]`)[1]?.split('}')[0];
      expect(palette).toContain(`--bg:${color};`);
    }
  });
});
