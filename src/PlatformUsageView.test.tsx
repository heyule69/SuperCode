// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, expect, it, vi } from 'vitest';
import { PlatformUsageIndicator } from './PlatformUsageView';
import type { PlatformUsage } from './platformUsage';
const state = vi.hoisted(() => ({ value: undefined as PlatformUsage | undefined, loading:false, stale:false, refresh:vi.fn(async()=>{}) }));
vi.mock('./platformUsage', async importOriginal => ({ ...await importOriginal<typeof import('./platformUsage')>(), usePlatformUsage: () => state }));
beforeEach(()=>{state.value=undefined;state.loading=false;state.stale=false;state.refresh.mockClear();});
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
it.each([true,false])('keeps the saved GLM plan entry visible while loading is %s and allows recovery',async loading=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;state.loading=loading;
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);const openStats=vi.fn();
  try{
    await act(async()=>root.render(<PlatformUsageIndicator agent="claude" connectionId="glm" revision="fixture" source={{providerId:'glm',providerName:'智谱 GLM',connectionName:'GLM Coding',planName:'GLM Coding Plan',mark:'Z'}} openStats={openStats}/>));
    const trigger=host.querySelector<HTMLButtonElement>('.platform-usage-trigger')!;
    expect(trigger.textContent).toContain(loading?'正在读取额度':'额度暂不可用');
    await act(async()=>trigger.click());
    expect(host.querySelector('.popover-heading')?.textContent).toContain('GLM Coding Plan');
    const refresh=host.querySelector<HTMLButtonElement>('[aria-label="刷新平台额度"]')!;
    expect(refresh.disabled).toBe(loading);
    expect(host.querySelectorAll('progress')).toHaveLength(0);
    if(!loading){await act(async()=>refresh.click());expect(state.refresh).toHaveBeenCalledWith(true);}
    await act(async()=>Array.from(host.querySelectorAll<HTMLButtonElement>('button')).find(button=>button.textContent==='所有平台')!.click());expect(openStats).toHaveBeenCalledOnce();
  }finally{await act(async()=>root.unmount());host.remove();}
});
it('labels retained percentages as old data when the current refresh failed',async()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;state.stale=true;
  state.value={agent:'claude',connectionId:'glm',providerName:'智谱 GLM',connectionName:'GLM Coding',queriedAt:1,balances:[],windows:[{label:'5 小时额度',remainingPercent:97}]};
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<PlatformUsageIndicator agent="claude" connectionId="glm" revision="fixture" openStats={()=>{}}/>));
    const trigger=host.querySelector<HTMLButtonElement>('.platform-usage-trigger')!;expect(trigger.textContent).toContain('剩余 97% · 上次数据');
    await act(async()=>trigger.click());expect(host.querySelector('[role="status"]')?.textContent).toContain('更新未成功');
    expect(host.querySelector('progress')?.value).toBe(97);
  }finally{await act(async()=>root.unmount());host.remove();}
});
