import { describe, expect, it } from 'vitest';
import { normalizeColor, normalizeThemeColors, resolvedThemeColors, themeColorVariables } from './themeColors';

describe('custom theme colors', () => {
  it('accepts hex colors while discarding unsafe or malformed saved values', () => {
    expect(normalizeColor(' #aBc ')).toBe('#AABBCC');
    expect(normalizeThemeColors({ light:{ accent:'#09f',background:'url(x)',foreground:'#123456',extra:'#fff' }, dark:{background:1}, unknown:{accent:'#fff'} })).toEqual({ light:{accent:'#0099FF',foreground:'#123456'} });
    for (const value of [null, [], 'red']) expect(normalizeThemeColors(value)).toEqual({});
  });
  it('keeps explicit foregrounds and provides readable defaults when only background changes', () => {
    expect(resolvedThemeColors('light', {background:'#000'}).foreground).toBe('#FFFFFF');
    expect(resolvedThemeColors('dark', {background:'#fff'}).foreground).toBe('#000000');
    expect(resolvedThemeColors('dark', {background:'#fff',foreground:'#f80'}).foreground).toBe('#FF8800');
  });
  it('generates native-compatible colors for related surfaces and leaves untouched palettes alone', () => {
    expect(themeColorVariables('light')).toEqual({});
    const accent = themeColorVariables('dark', {accent:'#a78bfa'});
    expect(accent['--accent']).toBe('#A78BFA');
    expect(accent['--bg']).toBeUndefined();
    const vars = themeColorVariables('light', {background:'#fcf6ed',foreground:'#382e25',accent:'#99754a'});
    expect(vars['--bg']).toBe('#FCF6ED');
    expect(vars['--window-bg']).toBe('#FCF6ED');
    expect(vars['--text']).toBe('#382E25');
    for (const value of Object.values(vars)) expect(value).toMatch(/^#[0-9A-F]{6}$/);
    expect(vars['--composer-bg']).not.toBe(vars['--bg']);
  });
});
