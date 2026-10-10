import { lazy, Suspense } from 'react';
import type { RpcEvent } from './types';
const RequestCard = lazy(() => import('./RequestCard'));

export default function QuestionDock({ requests, respond }: {
  requests: RpcEvent[];
  respond: (request: RpcEvent, result: unknown) => Promise<void>;
}) {
  const questions = requests.filter(request => request.method === 'item/tool/requestUserInput');
  if (!questions.length) return null;
  return <div className="question-dock" role="region" aria-label="待回答的问题">
    {questions.map(request => <Suspense key={String(request.id)} fallback={<div className="working-indicator">正在加载提问…</div>}>
      <RequestCard request={request} respond={result => respond(request, result)} />
    </Suspense>)}
  </div>;
}
