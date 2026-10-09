// Parser-blocking and independent of React: reloads must keep the saved palette.
(() => {
  let theme = 'light';
  try {
    const preferences = JSON.parse(localStorage.getItem('supercode.preferences') || '{}');
    const saved = preferences?.theme || localStorage.getItem('supercode.theme');
    if (['light', 'dark', 'graphite', 'sand', 'system'].includes(saved)) theme = saved;
  } catch { /* Storage can be unavailable; the default palette remains usable. */ }
  if (theme === 'system') theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  document.documentElement.dataset.theme = theme;
})();
