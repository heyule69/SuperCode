// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { call } from './api';
import { usePlatformUsage, type PlatformUsage } from './platformUsage';
vi.mock('./api',()=>({desktop:true,call:vi.fn()}));
function Indicator({connectionId}:{connectionId:string}){
  const {value,loading,stale,refresh}=usePlatformUsage('claude',connectionId,'same-credentials');
  return <div><span data-value>{value?.windows[0]?.remainingPercent??'unknown'}</span><span data-state>{loading?'loading':stale?'stale':'ready'}</span><button onClick={()=>void refresh(true)}>refresh</button></div>;
}
it('retains a failed refresh as marked old data and clears it immediately when the connection changes',async()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;vi.useFakeTimers();
  const fixture:PlatformUsage={agent:'claude',connectionId:'retained-glm',providerName:'GLM',connectionName:'GLM Coding',queriedAt:1,balances:[],windows:[{label:'5 小时额度',remainingPercent:61}]};
  vi.mocked(call).mockResolvedValueOnce(fixture).mockRejectedValue(new Error('network unavailable'));
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<Indicator connectionId="retained-glm"/>));
    await act(async()=>{await vi.advanceTimersByTimeAsync(121);});
    expect(host.querySelector('[data-value]')?.textContent).toBe('61');
    await act(async()=>host.querySelector<HTMLButtonElement>('button')!.click());
    expect(host.querySelector('[data-value]')?.textContent).toBe('61');expect(host.querySelector('[data-state]')?.textContent).toBe('stale');
    await act(async()=>root.render(<Indicator connectionId="other-account"/>));
    expect(host.querySelector('[data-value]')?.textContent).toBe('unknown');
    await act(async()=>{await vi.advanceTimersByTimeAsync(121);});
    expect(host.querySelector('[data-value]')?.textContent).toBe('unknown');
    expect(call).toHaveBeenCalledWith('get_platform_usage',{agent:'claude',connectionId:'other-account',refresh:false});
  }finally{await act(async()=>root.unmount());host.remove();vi.useRealTimers();}
});
