import { forwardRef, useImperativeHandle, useLayoutEffect, useRef, type ComponentProps } from 'react';

function resize(textarea: HTMLTextAreaElement | null) {
  if (!textarea) return;
  textarea.style.height = '0px';
  textarea.style.height = `${textarea.scrollHeight}px`;
}

/** Both chat surfaces grow with the draft, then scroll at their CSS height limit. */
export const ComposerTextarea = forwardRef<HTMLTextAreaElement, ComponentProps<'textarea'>>(function ComposerTextarea(props, forwardedRef) {
  const element = useRef<HTMLTextAreaElement>(null);
  useImperativeHandle(forwardedRef, () => element.current!, []);
  useLayoutEffect(() => { resize(element.current); }, [props.value, props.rows]);
  useLayoutEffect(() => {
    const textarea = element.current;
    if (!textarea) return;
    // A pane resize changes wrapping without changing the draft.
    let width = textarea.clientWidth;
    const observer = new ResizeObserver(() => {
      if (textarea.clientWidth === width) return;
      width = textarea.clientWidth;
      resize(textarea);
    });
    observer.observe(textarea);
    return () => observer.disconnect();
  }, []);
  return <textarea {...props} ref={element}/>;
});
