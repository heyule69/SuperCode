import { useEffect, useState } from 'react';
import { ChevronDown, RefreshCw, X } from 'lucide-react';
import { allowanceSummary, resetLabel, usePlatformUsage, type PlatformUsage } from './platformUsage';
import { useFloatingLayer } from './useFloatingLayer';

function Allowances({ value }: { value: PlatformUsage }) {
  return <div className="platform-allowances">{value.windows.map((window, index) => <div className="platform-window" key={`${window.label}:${index}`}><div><span>{window.label}</span><strong>剩余 {Math.floor(window.remainingPercent)}%</strong></div><progress max={100} value={window.remainingPercent} aria-label={`${window.label}剩余额度`} />{resetLabel(window.resetAt) ? <small>{resetLabel(window.resetAt)}</small> : null}</div>)}{value.balances.map((balance, index) => <div className="platform-balance" key={index}><span>{balance.label}</span><strong>{balance.currency === 'CNY' ? '¥' : balance.currency === 'USD' ? '$' : ''}{balance.amount.toFixed(2)}{balance.currency ? <small>{balance.currency}</small> : null}</strong></div>)}</div>;
}
export function PlatformUsageIndicator({ agent, connectionId, revision, openStats }: { agent: string; connectionId: string; revision: string; openStats: () => void }) {
  const { value, loading, refresh } = usePlatformUsage(agent, connectionId, revision);
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false));
  useEffect(() => { setOpen(false); }, [agent, connectionId]);
  if (!value || !allowanceSummary(value)) return null;
  const lowest = value.windows.length ? Math.min(...value.windows.map(window => window.remainingPercent)) : null;
  return <div className="platform-usage-indicator" ref={layer.root}><button type="button" className="platform-usage-trigger" aria-label={`查看 ${value.connectionName} 额度`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(old => !old)}><span>{value.providerName} · {allowanceSummary(value)}</span>{lowest != null ? <span className="platform-mini-meter" aria-hidden="true"><span style={{ width: `${lowest}%` }}/></span> : null}<ChevronDown size={11}/></button>{open ? <section className="platform-usage-popover usage-popover" role="dialog" aria-label={`${value.connectionName} 额度`}><div className="popover-heading"><div><strong>{value.connectionName}</strong>{value.planName ? <small>{value.planName}</small> : null}</div><button type="button" className="icon-button" aria-label="关闭平台额度" onClick={() => { setOpen(false); layer.restoreFocus(); }}><X size={13}/></button></div><Allowances value={value}/><div className="platform-usage-footer"><small>{new Date(value.queriedAt * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })} 更新</small><button type="button" className="icon-button" aria-label="刷新平台额度" disabled={loading} onClick={() => void refresh(true)}><RefreshCw size={13}/></button><button type="button" onClick={() => { setOpen(false); openStats(); }}>所有平台</button></div></section> : null}</div>;
}
