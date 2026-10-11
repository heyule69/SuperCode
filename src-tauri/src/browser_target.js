// Resolve only the observed target in the private CDP world; never click here.
(reference => {
  const state = globalThis.__supercodeBrowserObservation;
  const observed = state?.refs.get(reference);
  if (!observed || state.document !== document || state.url !== location.href) return {status:'changed'};
  let el = observed.element;
  if (!el.isConnected) {
    if (!observed.selector || (observed.root !== document && !observed.root.host?.isConnected)) return {status:'missing'};
    const matches = Array.from(observed.root.querySelectorAll(observed.selector)).filter(candidate => state.identify(candidate) === observed.identity);
    if (matches.length > 1) return {status:'ambiguous'};
    if (!matches.length) return {status:'missing'};
    el = observed.element = matches[0];
    observed.generation++;
  }
  // React/virtual lists may recycle the same DOM node for a different item.
  if (el.getRootNode() !== observed.root || state.identify(el) !== observed.identity) return {status:'changed'};
  if (state.disabled(el)) return {status:'disabled'};
  const r = el.getBoundingClientRect(), style = getComputedStyle(el);
  const left = Math.max(0,r.left), right = Math.min(innerWidth,r.right);
  const top = Math.max(0,r.top), bottom = Math.min(innerHeight,r.bottom);
  if (right <= left || bottom <= top || style.visibility === 'hidden' || style.display === 'none') return {status:'hidden'};
  const x = (left + right)/2, y = (top + bottom)/2;
  // Check every shadow boundary: an overlay outside a shadow root can cover it.
  let target = el, root = observed.root;
  for (;;) {
    const hit = root.elementFromPoint(x,y);
    if (!(hit === target || target.contains(hit) || target.shadowRoot?.contains(hit))) return {status:'blocked'};
    if (!root.host) break;
    target = root.host;
    root = target.getRootNode();
  }
  const editable = el.matches('textarea,[contenteditable="true"]') || (el.tagName === 'INPUT' && ['text','search','url','tel','email','password','number'].includes(el.type));
  return {status:'ready', x, y, rect:{x:r.x,y:r.y,width:r.width,height:r.height}, generation:observed.generation, textField:editable, editable:editable && !el.readOnly && el.getAttribute('aria-readonly') !== 'true', focused:observed.root.activeElement === el};
})
