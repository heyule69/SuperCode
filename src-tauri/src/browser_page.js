// Runs in a private CDP world. Page scripts cannot access observation references.
(() => {
  const refs = new Map();
  const nodes = [];
  const visit = root => {
    for (const el of root.querySelectorAll('*')) {
      if (nodes.length >= 200) break;
      if (el.shadowRoot) visit(el.shadowRoot);
      if (!el.matches('a[href],button,input,textarea,select,[role],[contenteditable="true"],[tabindex]')) continue;
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      if (r.width < 1 || r.height < 1 || r.bottom <= 0 || r.right <= 0 || r.top >= innerHeight || r.left >= innerWidth || style.visibility === 'hidden' || style.display === 'none') continue;
      const ref = `e${nodes.length + 1}`;
      const labels = Array.from(el.labels || []).map(x => x.innerText).join(' ');
      const role = el.getAttribute('role') || ({ A:'link', BUTTON:'button', TEXTAREA:'textbox', SELECT:'combobox', INPUT:el.type === 'checkbox' ? 'checkbox' : el.type === 'radio' ? 'radio' : 'textbox' }[el.tagName] || 'control');
      const name=(el.getAttribute('aria-label') || labels || el.innerText || el.getAttribute('placeholder') || el.title || '').slice(0,240);
      refs.set(ref, {element:el, name, rect:{x:r.x,y:r.y,width:r.width,height:r.height}});
      nodes.push({ ref, role, name, disabled:!!el.disabled, checked:typeof el.checked === 'boolean' ? el.checked : undefined, password:el.type === 'password' });
    }
  };
  visit(document);
  globalThis.__supercodeBrowserRefs = refs;
  return { url:location.href, title:document.title, text:(document.body?.innerText || '').slice(0,16000), width:innerWidth, height:innerHeight, elements:nodes, limitations:document.querySelector('iframe') ? 'Embedded frames are not included in this DOM snapshot; do not guess their controls.' : undefined };
})()
