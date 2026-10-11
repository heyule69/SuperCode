// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { PlatformUsageIndicator } from './PlatformUsageView';
import type { PlatformUsage } from './platformUsage';
const state = vi.hoisted(() => ({ value: undefined as PlatformUsage | undefined }));
vi.mock('./platformUsage', async importOriginal => ({ ...await importOriginal<typeof import('./platformUsage')>(), usePlatformUsage: () => ({ value: state.value, loading: false, refresh: vi.fn() }) }));
it('shows five-hour text and meter consistently while preserving weekly and MCP details', async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  state.value = { agent: 'claude', connectionId: 'glm', providerName: '智谱 GLM', connectionName: 'GLM Coding', queriedAt: 1, balances: [], windows: [
    { label: '周额度', remainingPercent: 87, windowDurationMins: 10080 },
    { label: '5 小时额度', remainingPercent: 97, windowDurationMins: 300 },
    { label: 'MCP 额度', remainingPercent: 1, scope: 'tool' },
  ] };
  const host = document.createElement('div'); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<PlatformUsageIndicator agent="claude" connectionId="glm" revision="fixture" openStats={() => {}}/>));
    const trigger = host.querySelector<HTMLButtonElement>('.platform-usage-trigger')!;
    expect(trigger.textContent).toBe('智谱 GLM · 5 小时额度 · 剩余 97%');
    expect(host.querySelector<HTMLElement>('.platform-mini-meter > span')?.style.width).toBe('97%');
    await act(async () => trigger.click());
    expect([...host.querySelectorAll('progress')].map(meter => meter.value)).toEqual([87, 97, 1]);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
