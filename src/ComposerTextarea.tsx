import { forwardRef, useImperativeHandle, useLayoutEffect, useRef, type ComponentProps } from 'react';

function resize(textarea: HTMLTextAreaElement | null, minHeight: number) {
  // Settings keep the chat mounted but hidden; measuring there would collapse
  // a long draft to zero before the chat becomes visible again.
  if (!textarea || textarea.closest('[hidden]')) return;
  // Shrinking the live control expands the adjacent chat viewport and clamps
  // its scroll position, even when the final input height stays unchanged.
  const measure = textarea.cloneNode() as HTMLTextAreaElement;
  measure.removeAttribute('id');
  measure.removeAttribute('name');
  measure.removeAttribute('autofocus');
  measure.setAttribute('aria-hidden', 'true');
  measure.tabIndex = -1;
  measure.value = textarea.value;
  Object.assign(measure.style, {
    position: 'fixed', visibility: 'hidden', pointerEvents: 'none',
    top: '0', left: '0', width: `${textarea.getBoundingClientRect().width}px`,
    height: '0', minHeight: '0', maxHeight: 'none', overflow: 'hidden',
  });
  textarea.parentElement!.append(measure);
  const height = Math.max(minHeight, measure.scrollHeight);
  measure.remove();
  textarea.style.height = `${height}px`;
}

/** Both chat surfaces grow with the draft, then scroll at their CSS height limit. */
export const ComposerTextarea = forwardRef<HTMLTextAreaElement, ComponentProps<'textarea'> & { minHeight?: number }>(function ComposerTextarea({ minHeight = 0, ...props }, forwardedRef) {
  const element = useRef<HTMLTextAreaElement>(null);
  useImperativeHandle(forwardedRef, () => element.current!, []);
  useLayoutEffect(() => { resize(element.current, minHeight); }, [props.value, props.rows, minHeight]);
  useLayoutEffect(() => {
    const textarea = element.current;
    if (!textarea) return;
    // A pane resize changes wrapping without changing the draft.
    let width = textarea.clientWidth;
    const observer = new ResizeObserver(() => {
      if (textarea.clientWidth === width) return;
      width = textarea.clientWidth;
      resize(textarea, minHeight);
    });
    observer.observe(textarea);
    return () => observer.disconnect();
  }, [minHeight]);
  return <textarea {...props} ref={element}/>;
});
