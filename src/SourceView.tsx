import { useEffect, useMemo, useRef } from 'react';

export default function SourceView({ text, line }: { text: string; line?: number }) {
  const root = useRef<HTMLDivElement>(null);
  const lines = useMemo(() => text.split('\n'), [text]);
  useEffect(() => { if (line) root.current?.querySelector(`[data-line="${line}"]`)?.scrollIntoView({ block: 'center', behavior: 'instant' }); }, [text, line]);
  return <div className="source-view" ref={root} tabIndex={0} aria-label="文件内容">{lines.slice(0, 5000).map((text, i) => <div key={i} data-line={i + 1} className={line === i + 1 ? 'source-selected' : ''}><span className="source-line-number" aria-hidden="true">{i + 1}</span><code>{text || ' '}</code></div>)}{lines.length > 5000 ? <p>仅显示前 5000 行</p> : null}</div>;
}
