export const MODEL_PAGE_SIZE = 50;

export function pageModels<T>(items: T[], requestedPage: number) {
  const pages = Math.max(1, Math.ceil(items.length / MODEL_PAGE_SIZE));
  const page = Math.max(0, Math.min(requestedPage, pages - 1));
  const start = page * MODEL_PAGE_SIZE;
  return { page, pages, start, items: items.slice(start, start + MODEL_PAGE_SIZE) };
}

export function ModelPagination({ count, page, change, label = '模型', disabled = false }: {
  count: number; page: number; change: (page: number) => void; label?: string; disabled?: boolean;
}) {
  if (count <= MODEL_PAGE_SIZE) return null;
  const last = Math.ceil(count / MODEL_PAGE_SIZE) - 1;
  return <div className="model-pagination" role="group" aria-label={`${label}分页`}>
    <button type="button" aria-label={`${label}上一页`} disabled={disabled || page === 0} onClick={() => change(page - 1)}>上一页</button>
    <span aria-live="polite">{page * MODEL_PAGE_SIZE + 1}–{Math.min(count, (page + 1) * MODEL_PAGE_SIZE)} / {count}</span>
    <button type="button" aria-label={`${label}下一页`} disabled={disabled || page === last} onClick={() => change(page + 1)}>下一页</button>
  </div>;
}
