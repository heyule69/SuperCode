import { lazy, Suspense } from 'react';
import type { FilePreview } from './WorkspacePanel';
import { CopyButton } from './CopyButton';
import { FileIcon } from './FileIcon';
const DiffView = lazy(() => import('./DiffView'));
const SourceView = lazy(() => import('./SourceView'));
const PlanDocument = lazy(() => import('./PlanDocument').then(module => ({ default: module.PlanDocument })));
export default function DocumentPanel({ preview, hidden, openFile }: { preview: FilePreview; hidden: boolean; openFile: (path: string, line?: number) => void }) {
  return <section className="context-panel workspace-panel document-panel" hidden={hidden} inert={hidden} aria-hidden={hidden} aria-label={preview.plan ? '计划文档' : '代码查看'}>
    <div className={`file-preview workspace-file-preview${preview.plan ? ' workspace-plan-preview' : ''}`}>
      {preview.plan ? <div className="document-plan-actions"><CopyButton text={preview.text} label="复制完整计划" iconOnly/></div> : <div className="file-preview-header"><FileIcon name={preview.name}/><span title={preview.name}>{preview.name}</span><CopyButton text={preview.text} label="复制文件内容" iconOnly/></div>}
      {preview.loading ? <div className="workspace-loading" role="status"><span className="pending-text">正在读取文件…</span></div> : preview.error ? <div className="error-banner" role="alert">{preview.error}</div> : <Suspense fallback={<pre>{preview.text}</pre>}>{preview.plan ? <PlanDocument text={preview.text} openFile={preview.openFile ?? openFile}/> : preview.diff ? <DiffView text={preview.text}/> : <SourceView text={preview.text} line={preview.line} name={preview.name}/>}</Suspense>}
    </div>
  </section>;
}
