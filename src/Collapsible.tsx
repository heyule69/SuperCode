import { useId, type ReactNode } from 'react';

/** Keep content mounted so opening and closing can animate without losing selection. */
export function Collapsible({ open, children, className = '', id }: { open: boolean; children: ReactNode; className?: string; id?: string }) {
  const generated = useId();
  return <div id={id ?? generated} className={`smooth-collapse ${open ? 'is-open' : ''} ${className}`} aria-hidden={!open} inert={!open}>
    <div className="smooth-collapse-inner">{children}</div>
  </div>;
}
