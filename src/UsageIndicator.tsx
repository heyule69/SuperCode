import { useState } from 'react';
import { ChevronDown, Shrink, X } from 'lucide-react';
import { desktop } from './api';
import { contextUsage, number, type TokenUsage } from './usage';
import { useFloatingLayer } from './useFloatingLayer';
export function UsageIndicator({ usage, busy, compact, openStats }: { usage?: TokenUsage; busy: boolean; compact: () => void; openStats: () => void }) {
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false));
  const { used, window, percent, label } = contextUsage(usage);
  const tokens = usage?.turn ?? (usage?.cumulative === false ? usage.total : usage?.last);
  const scope = usage?.turn || usage?.cumulative === false ? '本轮' : '最新调用';
  return <div ref={layer.root} className="usage-indicator"><button type="button" className="usage-trigger" onClick={() => setOpen(v => !v)} aria-expanded={open} aria-haspopup="dialog" aria-controls={open ? layer.id : undefined} title={used != null ? `最近一次上下文 ${number(used)}${window != null ? ` / ${number(window)}` : ''} Token` : '上下文用量尚未返回'}><span aria-hidden="true" className={`context-ring ${percent == null ? 'context-unknown' : ''} ${percent != null && percent >= 85 ? 'context-high' : ''}`} style={{ '--context-percent': `${percent ?? 0}%` } as React.CSSProperties} />{label}<ChevronDown size={11} /></button>{open ? <div className="usage-popover" role="dialog" aria-label="上下文与用量" id={layer.id}><div className="popover-heading"><strong>上下文</strong><button type="button" className="icon-button" onClick={() => { setOpen(false); layer.restoreFocus(); }} aria-label="关闭上下文"><X size={13} /></button></div><div className="usage-context"><strong>{number(used)}</strong><span>{window != null ? ` / ${number(window)}` : ''} Token</span></div>{percent != null ? <progress aria-label="上下文占用" value={percent} max={100} /> : <small>{used != null ? '已显示最近一次实际用量；Agent 未提供上下文上限' : 'Agent 尚未提供上下文用量'}</small>}<dl><div><dt>{scope}输入</dt><dd>{number(tokens?.inputTokens)}</dd></div><div><dt>缓存读取</dt><dd>{number(tokens?.cachedInputTokens)}</dd></div><div><dt>{scope}输出</dt><dd>{number(tokens?.outputTokens)}</dd></div>{usage?.costUsd != null ? <div><dt>本轮费用</dt><dd>${usage.costUsd.toFixed(4)}</dd></div> : null}</dl><button type="button" className="quiet-button" disabled={busy || !desktop} onClick={() => { compact(); setOpen(false); layer.restoreFocus(); }}><Shrink size={13} />压缩上下文</button><button type="button" className="text-button" onClick={() => { openStats(); setOpen(false); }}>查看用量统计</button></div> : null}</div>;
}
