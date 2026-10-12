import { lazy, Suspense } from 'react';
import './plan-request.css';
const Markdown = lazy(() => import('./Markdown'));

export function PlanDocument({ text, openFile, streaming = false, preview = false }: { text: string; openFile?: (path: string, line?: number) => void; streaming?: boolean; preview?: boolean }) {
  return <div className="plan-document message-content" role="region" aria-label={preview ? '计划摘要' : '完整计划文档'} tabIndex={preview ? undefined : 0}>
    <Suspense fallback={<div className="plan-document-loading">正在排版计划文档…</div>}><Markdown text={text} openFile={openFile} streaming={streaming} /></Suspense>
  </div>;
}
