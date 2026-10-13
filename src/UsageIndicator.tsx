import { useId, useLayoutEffect, useRef, useState } from 'react';
import { contextUsage, number, type TokenUsage } from './usage';
import { contextNumber } from './modelContext';
import './context-usage.css';
export function UsageIndicator({ usage, contextWindow }: { usage?: TokenUsage; contextWindow?: number }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const tooltip = useRef<HTMLDivElement>(null);
  const tooltipId = useId();
  const showTooltip = !dismissed && (hovered || focused);
  const { used, window: capacity, percent, label } = contextUsage(usage, contextWindow);
  const ringPercent = used != null && capacity != null ? Math.min(100, used / capacity * 100) : undefined;
  useLayoutEffect(() => {
    const el = tooltip.current;
    if (!showTooltip || !el) return;
    el.style.setProperty('--tooltip-shift', '0px');
    const bounds = el.getBoundingClientRect();
    el.style.setProperty('--tooltip-shift', `${Math.max(12 - bounds.left, Math.min(0, window.innerWidth - 12 - bounds.right))}px`);
  }, [showTooltip, used, capacity, percent]);
  const summary = percent != null ? `${used && percent === 0 ? '<1' : percent}% 已用（剩余 ${used && percent === 0 ? '>99' : 100 - percent}%）` : '上下文用量尚未完整返回';
  return <div className="usage-indicator" onMouseEnter={() => { setHovered(true); setDismissed(false); }} onMouseLeave={() => setHovered(false)}>
    <span className="usage-trigger" tabIndex={0} role={ringPercent != null ? 'meter' : 'img'} onFocus={() => { setFocused(true); setDismissed(false); }} onBlur={() => setFocused(false)} onKeyDown={event => { if (event.key === 'Escape') setDismissed(true); }} aria-label={used != null ? `最近一次上下文 ${number(used)}${capacity != null ? ` / ${number(capacity)}` : ''} Token` : '上下文用量尚未返回'} aria-valuemin={ringPercent != null ? 0 : undefined} aria-valuemax={ringPercent != null ? 100 : undefined} aria-valuenow={ringPercent} aria-valuetext={percent != null ? summary : undefined} aria-describedby={showTooltip ? tooltipId : undefined}>
      <span aria-hidden="true" className={`context-ring ${percent == null ? 'context-unknown' : ''} ${percent != null && percent >= 85 ? 'context-high' : ''}`} style={{ '--context-percent': `${ringPercent ?? 0}%` } as React.CSSProperties} />{label}
    </span>
    {showTooltip ? <div ref={tooltip} role="tooltip" id={tooltipId} className="context-tooltip">{percent != null ? <div>{summary}</div> : null}<div>{used != null ? `已用 ${contextNumber(used)} Token` : '使用量暂未返回'}{capacity != null ? `，共 ${contextNumber(capacity)}` : '，上限暂未返回'}</div></div> : null}
  </div>;
}
