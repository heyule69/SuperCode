import { useId, useLayoutEffect, useRef, useState } from 'react';
import { ChevronDown, Shrink, X } from 'lucide-react';
import { desktop } from './api';
import { contextUsage, number, type TokenUsage } from './usage';
import { useFloatingLayer } from './useFloatingLayer';
import { contextNumber } from './modelContext';
import './context-usage.css';
export function UsageIndicator({ usage, contextWindow, busy, compact, openStats }: { usage?: TokenUsage; contextWindow?: number; busy: boolean; compact: () => void; openStats: () => void }) {
  const [open, setOpen] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const tooltip = useRef<HTMLDivElement>(null);
  const tooltipId = useId();
  const showTooltip = !open && !dismissed && (hovered || focused);
  const layer = useFloatingLayer(open, () => setOpen(false));
  const { used, window: capacity, percent, label } = contextUsage(usage, contextWindow);
  const ringPercent = used != null && capacity != null ? Math.min(100, used / capacity * 100) : undefined;
  useLayoutEffect(() => {
    const el = tooltip.current;
    if (!showTooltip || !el) return;
    el.style.setProperty('--tooltip-shift', '0px');
    const bounds = el.getBoundingClientRect();
    el.style.setProperty('--tooltip-shift', `${Math.max(12 - bounds.left, Math.min(0, window.innerWidth - 12 - bounds.right))}px`);
  }, [showTooltip, used, capacity, percent]);
  const tokens = usage?.turn ?? (usage?.cumulative === false ? usage.total : usage?.last);
  const scope = usage?.turn || usage?.cumulative === false ? '本轮' : '最新调用';
  const summary = percent != null ? `${used && percent === 0 ? '<1' : percent}% 已用（剩余 ${used && percent === 0 ? '>99' : 100 - percent}%）` : '上下文用量尚未完整返回';
  return <div ref={layer.root} className="usage-indicator" onMouseEnter={() => { setHovered(true); setDismissed(false); }} onMouseLeave={() => setHovered(false)}>
    <button type="button" className="usage-trigger" onClick={() => setOpen(v => !v)} onFocus={() => { setFocused(true); setDismissed(false); }} onBlur={() => setFocused(false)} onKeyDown={event => { if (event.key === 'Escape' && !open) setDismissed(true); }} aria-label={used != null ? `最近一次上下文 ${number(used)}${capacity != null ? ` / ${number(capacity)}` : ''} Token` : '上下文用量尚未返回'} aria-expanded={open} aria-haspopup="dialog" aria-controls={open ? layer.id : undefined} aria-describedby={showTooltip ? tooltipId : undefined}>
      <span role="meter" aria-label="上下文占用" aria-valuemin={0} aria-valuemax={100} aria-valuenow={ringPercent} aria-valuetext={percent != null ? summary : '上下文占比尚未返回'} className={`context-ring ${percent == null ? 'context-unknown' : ''} ${percent != null && percent >= 85 ? 'context-high' : ''}`} style={{ '--context-percent': `${ringPercent ?? 0}%` } as React.CSSProperties} />{label}<ChevronDown size={11} />
    </button>
    {showTooltip ? <div ref={tooltip} role="tooltip" id={tooltipId} className="context-tooltip"><div className="context-tooltip-heading">背景信息窗口：</div><div>{summary}</div><div>{used != null ? `已用 ${contextNumber(used)} 标记` : '使用量暂未返回'}{capacity != null ? `，共 ${contextNumber(capacity)}` : '，上限暂未返回'}</div></div> : null}
    {open ? <div className="usage-popover" role="dialog" aria-label="上下文与用量" id={layer.id}><div className="popover-heading"><strong>上下文</strong><button type="button" className="icon-button" onClick={() => { setOpen(false); layer.restoreFocus(); }} aria-label="关闭上下文"><X size={13} /></button></div><div className="usage-context"><strong>{number(used)}</strong><span>{capacity != null ? ` / ${number(capacity)}` : ''} Token</span></div>{percent != null ? <progress aria-label="上下文占用" value={percent} max={100} /> : <small>{used != null ? '已显示最近一次实际用量；Agent 未提供上下文上限' : 'Agent 尚未提供上下文用量'}</small>}<dl><div><dt>{scope}输入</dt><dd>{number(tokens?.inputTokens)}</dd></div><div><dt>缓存读取</dt><dd>{number(tokens?.cachedInputTokens)}</dd></div><div><dt>{scope}输出</dt><dd>{number(tokens?.outputTokens)}</dd></div>{usage?.costUsd != null ? <div><dt>本轮费用</dt><dd>${usage.costUsd.toFixed(4)}</dd></div> : null}</dl><button type="button" className="quiet-button" disabled={busy || !desktop} onClick={() => { compact(); setOpen(false); layer.restoreFocus(); }}><Shrink size={13} />压缩上下文</button><button type="button" className="text-button" onClick={() => { openStats(); setOpen(false); }}>查看用量统计</button></div> : null}
  </div>;
}
