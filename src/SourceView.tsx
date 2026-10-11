import { useEffect, useMemo, useRef } from 'react';
import { highlightedLines } from './codeHighlight';
import './code-syntax.css';

export default function SourceView({ text, line, name = '' }: { text: string; line?: number; name?: string }) {
  const root = useRef<HTMLDivElement>(null);
  const lines = useMemo(() => highlightedLines(text.split('\n').slice(0, 5000).join('\n'), name), [text, name]);
  useEffect(() => { if (line) root.current?.querySelector(`[data-line="${line}"]`)?.scrollIntoView({ block: 'center', behavior: 'instant' }); }, [text, line]);
  return <div className="source-view code-syntax" ref={root} tabIndex={0} aria-label="文件内容">{lines.map((html, i) => <div key={i} data-line={i + 1} className={line === i + 1 ? 'source-selected' : ''}><span className="source-line-number" aria-hidden="true">{i + 1}</span><code dangerouslySetInnerHTML={{__html:html||' '}}/></div>)}{text.split('\n').length > 5000 ? <p>仅显示前 5000 行</p> : null}</div>;
}
