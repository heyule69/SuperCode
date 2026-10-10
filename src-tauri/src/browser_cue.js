// Browser-local cue; it never changes the Windows cursor.
((point, active) => {
  let cue = document.getElementById('__supercode-browser-cue');
  if (!active) { cue?.remove(); return; }
  if (!cue) {
    cue = document.createElement('div'); cue.id = '__supercode-browser-cue';
    Object.assign(cue.style,{position:'fixed',inset:'0',pointerEvents:'none',zIndex:'2147483647',boxShadow:'inset 0 0 18px rgba(86,165,237,.28), inset 0 0 2px rgba(114,186,245,.7)'});
    const pointer = document.createElement('div'); pointer.dataset.pointer = 'true';
    Object.assign(pointer.style,{position:'absolute',width:'100px',height:'100px',display:'none',transform:'translate(-50%,-50%)',background:'radial-gradient(circle,rgba(69,153,228,.33),rgba(69,153,228,.12) 35%,transparent 70%)',transition:'left 120ms ease-out, top 120ms ease-out'});
    const svg = document.createElementNS('http://www.w3.org/2000/svg','svg');
    svg.setAttribute('width','29'); svg.setAttribute('height','32'); svg.setAttribute('viewBox','0 0 29 32');
    Object.assign(svg.style,{position:'absolute',left:'49px',top:'48px'});
    const path = document.createElementNS('http://www.w3.org/2000/svg','path');
    for (const [key,value] of Object.entries({d:'M3 2 Q1 1 1.7 4 L8 27 Q9 30 10.5 27 L14 19 L23 15 Q26 13.5 23 12 Z',fill:'#111315',stroke:'#d5d8db','stroke-width':'2.4','stroke-linejoin':'round'})) path.setAttribute(key,value);
    svg.append(path); pointer.append(svg);
    cue.append(pointer); document.documentElement.append(cue);
  }
  if (point) {
    const pointer = cue.querySelector('[data-pointer]');
    pointer.style.display = 'block'; pointer.style.left = `${point.x}px`; pointer.style.top = `${point.y}px`;
    if (point.click) {
      const ring = document.createElement('div'); Object.assign(ring.style,{position:'absolute',left:`${point.x}px`,top:`${point.y}px`,width:'16px',height:'16px',margin:'-8px',border:'1px solid rgba(130,203,255,.7)',borderRadius:'50%'}); cue.append(ring);
      ring.animate([{transform:'scale(.7)',opacity:.8},{transform:'scale(2.4)',opacity:0}],{duration:430}).finished.then(()=>ring.remove());
    }
  }
})
