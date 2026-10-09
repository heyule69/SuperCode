import { useMemo } from 'react';
import { diffLines } from './diff';
export default function DiffView({ text }: { text: string }) {
  const lines = useMemo(() => diffLines(text), [text]);
  return <div className="diff-view" tabIndex={0} aria-label="代码差异">{lines.map((line, i) => <div key={i} className={`diff-${line.kind}`}><span className="diff-line-number">{line.old ?? ''}</span><span className="diff-line-number">{line.next ?? ''}</span><code>{line.text || ' '}</code></div>)}{text.split('\n').length > 5000 ? <p>差异超过 5000 行，请在编辑器中查看完整文件。</p> : null}</div>;
}
