import { useEffect, useState } from 'react';
import { ChevronDown, RefreshCw, X } from 'lucide-react';
import { allowanceSummary, primaryAllowance, resetLabel, supportsPlatformUsage, usePlatformUsage, type PlatformUsage } from './platformUsage';
import { useFloatingLayer } from './useFloatingLayer';
import type { ModelSource } from './types';

function Allowances({ value }: { value: PlatformUsage }) {
  return <div className="platform-allowances">{value.windows.map((window, index) => <div className="platform-window" key={`${window.label}:${index}`}><div><span>{window.label}</span><strong>剩余 {Math.floor(window.remainingPercent)}%</strong></div><progress max={100} value={window.remainingPercent} aria-label={`${window.label}剩余额度`} />{resetLabel(window.resetAt) ? <small>{resetLabel(window.resetAt)}</small> : null}</div>)}{value.balances.map((balance, index) => <div className="platform-balance" key={index}><span>{balance.label}</span><strong>{balance.currency === 'CNY' ? '¥' : balance.currency === 'USD' ? '$' : ''}{balance.amount.toFixed(2)}{balance.currency ? <small>{balance.currency}</small> : null}</strong></div>)}</div>;
}
export function PlatformUsageIndicator({ agent, connectionId, revision, source, openStats }: { agent: string; connectionId: string; revision: string; source?: ModelSource; openStats: () => void }) {
  const { value, loading, stale, refresh } = usePlatformUsage(agent, connectionId, revision);
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false));
  useEffect(() => { setOpen(false); }, [agent, connectionId]);
  const summary = value ? allowanceSummary(value) : '';
  if (!summary && !supportsPlatformUsage(agent, source)) return null;
  const providerName = value?.providerName || source?.providerName || '套餐';
  const connectionName = value?.connectionName || source?.connectionName || providerName;
  const planName = value?.planName || source?.planName;
  const label = `${providerName} · ${summary ? summary + (stale ? ' · 上次数据' : '') : loading ? '正在读取额度…' : '额度暂不可用'}`;
  const remaining = value ? primaryAllowance(value)?.remainingPercent : undefined;
  return <div className="platform-usage-indicator" ref={layer.root}><button type="button" className="platform-usage-trigger" title={label} aria-label={`查看 ${connectionName} 额度`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(old => !old)}><span>{label}</span>{remaining != null ? <span className="platform-mini-meter" aria-hidden="true"><span style={{ width: `${remaining}%` }}/></span> : null}<ChevronDown size={11}/></button>{open ? <section className="platform-usage-popover usage-popover" role="dialog" aria-label={`${connectionName} 额度`}><div className="popover-heading"><div><strong>{connectionName}</strong>{planName ? <small>{planName}</small> : null}</div><button type="button" className="icon-button" aria-label="关闭平台额度" onClick={() => { setOpen(false); layer.restoreFocus(); }}><X size={13}/></button></div>{summary && value ? <><Allowances value={value}/>{stale ? <p className="platform-usage-status" role="status">本次更新未成功，正在显示上次读取的数据。</p> : null}</> : <p className="platform-usage-status" role="status">{loading ? '正在读取套餐额度…' : '暂未读取到额度数据，套餐连接仍保留。可以刷新后重试。'}</p>}<div className="platform-usage-footer"><small>{value ? `${new Date(value.queriedAt * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })} 更新` : loading ? '读取中' : '等待重新读取'}</small><button type="button" className="icon-button" aria-label="刷新平台额度" disabled={loading} onClick={() => void refresh(true)}><RefreshCw size={13} className={loading ? 'spin' : undefined}/></button><button type="button" onClick={() => { setOpen(false); openStats(); }}>所有平台</button></div></section> : null}</div>;
}
