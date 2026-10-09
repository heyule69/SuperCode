export function scrollToLatest(area: HTMLElement | null, behavior: ScrollBehavior = 'instant') {
  area?.scrollTo({ top: area.scrollHeight, behavior });
}

export function revealScrollItem(area: HTMLElement | null, item: HTMLElement | null) {
  if (!area || !item) return;
  const viewport = area.getBoundingClientRect();
  const target = item.getBoundingClientRect();
  const top = viewport.top + area.clientTop;
  const bottom = top + area.clientHeight;
  if (target.top < top) area.scrollTop += target.top - top;
  else if (target.bottom > bottom) area.scrollTop += target.bottom - bottom;
}

export function scrollToConversationTurn(area: HTMLElement, item: HTMLElement, behavior: ScrollBehavior) {
  let frame = 0;
  let stopped = false;
  let corrections = 0;
  let previousTop = area.scrollTop;
  const started = performance.now();
  let stableSince = started;
  const targetTop = () => {
    const rect = item.getBoundingClientRect();
    const top = area.scrollTop + rect.top - area.getBoundingClientRect().top - (area.clientHeight / 2 - Math.min(40, rect.height / 2));
    return Math.max(0, Math.min(top, area.scrollHeight - area.clientHeight));
  };
  const stop = () => {
    if (stopped) return;
    stopped = true;
    cancelAnimationFrame(frame);
    for (const event of ['wheel', 'touchstart', 'pointerdown', 'keydown']) area.removeEventListener(event, stop);
  };
  const settle = (time: number) => {
    if (stopped) return;
    if (!item.isConnected || !area.clientHeight || time - started > 5000) { stop(); return; }
    if (Math.abs(area.scrollTop - previousTop) > .5) {
      previousTop = area.scrollTop;
      stableSince = time;
    } else if (behavior === 'instant' || time - stableSince >= 100) {
      // content-visibility and lazy Markdown can change earlier heights during travel.
      const top = targetTop();
      if (Math.abs(top - area.scrollTop) <= 2 || corrections >= 3) { stop(); return; }
      corrections++;
      area.scrollTo({ top, behavior });
      previousTop = area.scrollTop;
      stableSince = time;
    }
    frame = requestAnimationFrame(settle);
  };
  for (const event of ['wheel', 'touchstart', 'pointerdown', 'keydown']) area.addEventListener(event, stop, { passive: true });
  area.scrollTo({ top: targetTop(), behavior });
  frame = requestAnimationFrame(settle);
  return stop;
}
