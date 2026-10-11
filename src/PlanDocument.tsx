import { lazy, Suspense } from 'react';
import './plan-request.css';
const Markdown = lazy(() => import('./Markdown'));

export function PlanDocument({ text, openFile }: { text: string; openFile?: (path: string, line?: number) => void }) {
  return <div className="plan-document message-content" role="region" aria-label="完整计划文档" tabIndex={0}>
    <Suspense fallback={<div className="plan-document-loading">正在排版计划文档…</div>}><Markdown text={text} openFile={openFile} /></Suspense>
  </div>;
}
