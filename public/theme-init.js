// Parser-blocking and independent of React: reloads must keep the saved palette.
(() => {
  let theme = 'light';
  let palettes;
  try {
    const preferences = JSON.parse(localStorage.getItem('supercode.preferences') || '{}');
    palettes = preferences?.themeColors;
    const saved = preferences?.theme || localStorage.getItem('supercode.theme');
    if (['light', 'dark', 'graphite', 'sand', 'system'].includes(saved)) theme = saved;
  } catch { /* Storage can be unavailable; the default palette remains usable. */ }
  if (theme === 'system') theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  document.documentElement.dataset.theme = theme;
  const colors = palettes?.[theme];
  for (const [key, variable] of [['background', '--bg'], ['foreground', '--text'], ['accent', '--accent']]) {
    const value = colors?.[key];
    if (typeof value !== 'string' || !/^#(?:[\da-f]{3}|[\da-f]{6})$/i.test(value.trim())) continue;
    const hex = value.trim().slice(1);
    document.documentElement.style.setProperty(variable, `#${hex.length === 3 ? [...hex].map(c => c + c).join('') : hex}`.toUpperCase());
  }
})();
