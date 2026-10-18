// Runs in a private CDP world. Page scripts cannot access observation references.
(() => {
  const refs = new Map();
  const nodes = [];
  const images = [];
  const controls = 'a[href],button,input,textarea,select,[role],[contenteditable="true"],[tabindex]';
  const nameOf = el => (el.getAttribute('aria-label') || Array.from(el.labels || []).map(x => x.innerText).join(' ') || el.innerText || el.getAttribute('placeholder') || el.title || '').replace(/\s+/g, ' ').trim();
  const roleOf = el => el.getAttribute('role') || ({ A:'link', BUTTON:'button', TEXTAREA:'textbox', SELECT:'combobox', INPUT:el.type === 'checkbox' ? 'checkbox' : el.type === 'radio' ? 'radio' : 'textbox' }[el.tagName] || 'control');
  // Compare meaning, not old geometry. Keep values/passwords out of identities.
  const identify = el => {
    const identity = JSON.stringify([el.tagName, roleOf(el), nameOf(el), el.id, el.getAttribute('name'), el.getAttribute('type'), el.getAttribute('href'), el.matches('a[href]') ? el.href : null, el.getAttribute('target'), el.getAttribute('download'), el.getAttribute('formaction'), el.getAttribute('aria-controls'), el.getAttribute('data-id'), el.getAttribute('data-key'), el.form?.id, el.form?.action, el.form?.method]);
    return identity.length <= 8192 ? identity : null;
  };
  const disabled = el => el.matches(':disabled') || !!el.closest('[aria-disabled="true"]');
  let visited = 0;
  const visit = root => {
    for (const el of root.querySelectorAll('*')) {
      if (++visited > 6000) break;
      if (el.shadowRoot) visit(el.shadowRoot);
      if (!el.matches(controls) && el.tagName !== 'IMG') continue;
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      if (r.width < 1 || r.height < 1 || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth || style.visibility === 'hidden' || style.display === 'none') continue;
      if (el.tagName === 'IMG' && images.length < 80) {
        // Report only the source the browser currently displays, not guessed originals.
        const src = el.currentSrc || el.src;
        if (/^https?:\/\//i.test(src) && src.length <= 4096) images.push({ url:src, alt:el.alt.slice(0,240), width:r.width, height:r.height, naturalWidth:el.naturalWidth, naturalHeight:el.naturalHeight, loaded:el.complete && el.naturalWidth > 0 });
      }
      if (!el.matches(controls) || nodes.length >= 200) continue;
      const identity = identify(el);
      if (!identity) continue;
      const ref = `e${nodes.length + 1}`;
      const role = roleOf(el), name = nameOf(el).slice(0,240);
      // A detached node can only be rebound by a strong, unique key in the same root.
      const selector = el.id ? `#${CSS.escape(el.id)}` : el.matches('a[href]') ? `a[href="${CSS.escape(el.getAttribute('href'))}"]` : el.form && el.getAttribute('name') ? `${el.tagName.toLowerCase()}[name="${CSS.escape(el.getAttribute('name'))}"]` : null;
      refs.set(ref, {element:el, root, identity, selector, generation:0});
      nodes.push({ ref, role, name, disabled:disabled(el), checked:typeof el.checked === 'boolean' ? el.checked : undefined, password:el.type === 'password' });
    }
  };
  visit(document);
  globalThis.__supercodeBrowserObservation = {refs, identify, disabled, controls, document, url:location.href};
  return { url:location.href, title:document.title, text:(document.body?.innerText || '').slice(0,16000), width:innerWidth, height:innerHeight, scrollX, scrollY, elements:nodes, images, truncated:visited > 6000 || nodes.length >= 200 || images.length >= 80, limitations:document.querySelector('iframe') ? 'Embedded frames are not included in this DOM snapshot; do not guess their controls.' : undefined };
})()
