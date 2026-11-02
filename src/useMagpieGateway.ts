import { useEffect, useState } from 'react';
import { call, desktop } from './api';

export type MagpieGateway = 'checking' | 'reachable' | 'offline' | 'remote' | 'unconfigured' | 'unknown';
export function magpieGatewayText(state?: MagpieGateway) {
  return state === 'checking' ? '正在检测 Magpie 网关…'
    : state === 'reachable' ? 'Magpie 网关可连接，窗口关闭后仍可能在后台运行。'
    : state === 'offline' ? 'Magpie 网关未连接。模型目录已保留，请打开 Magpie 后再发送。'
    : state === 'remote' ? '使用远程 Magpie 网关，无需本机 Magpie 窗口运行；连接情况以实际请求为准。'
    : state === 'unconfigured' ? '尚未添加 Magpie 连接。'
    : state === 'unknown' ? '暂时无法确认 Magpie 网关状态，发送前会重新检查。'
    : 'Magpie 模型来自已保存目录，调用前会检查网关。';
}

// Inspect on opening only. Never launch Magpie or poll the model directory.
export function useMagpieGateway(enabled: boolean, agent?: string, connectionId?: string): MagpieGateway | undefined {
  const key = `${agent ?? ''}\0${connectionId ?? ''}`;
  const [result, setResult] = useState<{ key:string; state:MagpieGateway }>();
  useEffect(() => {
    if (!enabled || !desktop) { setResult(undefined); return; }
    let disposed = false;
    setResult({ key, state:'checking' });
    void call<{ state?: string }>('get_magpie_status', { agent:agent ?? null, connectionId:connectionId ?? null }).then(value => {
      const state = ['reachable', 'offline', 'remote', 'unconfigured'].includes(value?.state ?? '') ? value.state as MagpieGateway : 'unknown';
      if (!disposed) setResult({ key, state });
    }).catch(() => { if (!disposed) setResult({ key, state:'unknown' }); });
    return () => { disposed = true; };
  }, [enabled, agent, connectionId, key]);
  return enabled && desktop ? result?.key === key ? result.state : 'checking' : undefined;
}
