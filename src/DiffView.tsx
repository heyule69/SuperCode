import { useMemo } from 'react';
import { diffFiles } from './diff';
import { ReviewDiff } from './ChangeReview';
import { FileIcon } from './FileIcon';
export default function DiffView({ text }: { text: string }) {
  const files = useMemo(() => diffFiles(text), [text]);
  return <div className="change-review is-wrapped change-preview" tabIndex={0} aria-label="代码差异">{files.length ? files.map(file => <section className="change-file" key={file.path}><div className="change-file-header"><FileIcon name={file.path}/><span className="change-file-path" title={file.path}>{file.path}</span><span className="change-count"><span className="change-added">+{file.added}</span><span className="change-removed">−{file.removed}</span></span></div><ReviewDiff text={file.diff} path={file.path}/></section>) : <ReviewDiff text={text}/>}</div>;
}
