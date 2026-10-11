// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, it, vi } from 'vitest';
import { UsageIndicator } from './UsageIndicator';
vi.mock('./api', () => ({ desktop: true }));
(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
const host = document.createElement('div');
document.body.append(host);
const root = createRoot(host);
afterEach(async () => { await act(async () => root.render(null)); });
it('updates visible tokens while running and explains a missing capacity without claiming missing usage', async () => {
  const props = { busy: true, compact: vi.fn(), openStats: vi.fn() };
  await act(async () => root.render(<UsageIndicator {...props} usage={{ total: {}, last: {}, contextTokens: 12345, cumulative: false }}/>));
  const button = host.querySelector<HTMLButtonElement>('.usage-trigger')!;
  expect(button.textContent).toBe('上下文 12.3K');
  await act(async () => button.click());
  expect(host.textContent).toContain('12,345');
  expect(host.textContent).toContain('已显示最近一次实际用量');
  expect(button.title).toBe('最近一次上下文 12,345 Token');
  expect(host.textContent).not.toContain('尚未提供上下文用量');
  await act(async () => root.render(<UsageIndicator {...props} busy={false} usage={{ total: {}, last: {}, contextTokens: 24000, modelContextWindow: 128000 }}/>));
  expect(button.textContent).toBe('上下文 19%');
  expect(host.querySelector('progress')?.value).toBe(19);
});
