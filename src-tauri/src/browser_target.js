// Recheck the observed control before dispatching native browser input.
(reference => {
  const observed = globalThis.__supercodeBrowserRefs?.get(reference);
  const el = observed?.element;
  if (!el?.isConnected || el.disabled || el.readOnly) return null;
  const r = el.getBoundingClientRect();
  const labels = Array.from(el.labels || []).map(x => x.innerText).join(' ');
  const name = (el.getAttribute('aria-label') || labels || el.innerText || el.getAttribute('placeholder') || el.title || '').slice(0,240);
  if (name !== observed.name || ['x','y','width','height'].some(key => Math.abs(r[key] - observed.rect[key]) > 2)) return null;
  const x = Math.max(0,r.left) + (Math.min(innerWidth,r.right) - Math.max(0,r.left))/2;
  const y = Math.max(0,r.top) + (Math.min(innerHeight,r.bottom) - Math.max(0,r.top))/2;
  const hit = el.getRootNode().elementFromPoint(x,y);
  if (!(r.width > 0 && r.height > 0 && x >= 0 && x < innerWidth && y >= 0 && y < innerHeight && (hit === el || el.contains(hit) || el.shadowRoot?.contains(hit)))) return null;
  return {x,y,editable:el.matches('input:not([type=checkbox]):not([type=radio]):not([type=button]):not([type=submit]):not([type=file]),textarea,[contenteditable=true]')};
})
