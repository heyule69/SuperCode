import { memo, useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { ChevronRight } from 'lucide-react';
import { ProviderIcon } from './AgentIcon';
import type { QuotaConnection } from './platformUsage';

export const quotaRowHeight = 68;
export const agentNames: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', opencode: 'OpenCode', pi: 'Pi' };
export const quotaConnectionKey = (connection: QuotaConnection) => JSON.stringify([connection.agent, connection.connectionId]);

export function quotaListWindow(count: number, scrollTop: number, height: number) {
  const start = Math.min(Math.max(0, count - 1), Math.max(0, Math.floor(scrollTop / quotaRowHeight) - 4));
  const end = Math.min(count, Math.max(start, Math.ceil((Math.max(0, scrollTop) + Math.max(0, height)) / quotaRowHeight) + 4));
  return { start, end };
}

const ConnectionRow = memo(function ConnectionRow({ connection, index, count, id, selected, focused, choose }: {
  connection: QuotaConnection; index: number; count: number; id: string; selected: boolean; focused: boolean; choose: (key: string) => void;
}) {
  const subtitle = `${agentNames[connection.agent] ?? connection.agent} · ${connection.source.planName || connection.source.providerName}`;
  return <button id={id} type="button" role="option" aria-selected={selected} aria-posinset={index + 1} aria-setsize={count} tabIndex={-1}
    className={`quota-connection${focused ? ' keyboard-focus' : ''}`} style={{ top: index * quotaRowHeight }}
    title={`${connection.connectionName}\n${subtitle}`} onMouseDown={event => event.preventDefault()} onClick={() => choose(quotaConnectionKey(connection))}>
    <span className="quota-brand"><ProviderIcon provider={connection.source.providerId} name={connection.connectionName} mark={connection.source.mark}/></span>
    <span><strong>{connection.connectionName}</strong><small>{subtitle}</small></span><ChevronRight size={13}/>
  </button>;
});

export const QuotaConnections = memo(function QuotaConnections({ connections, activeKey, choose }: {
  connections: QuotaConnection[]; activeKey: string; choose: (key: string) => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const frame = useRef(0);
  const id = useId();
  const [viewport, setViewport] = useState({ top: 0, height: 480 });
  const [focusKey, setFocusKey] = useState(activeKey);
  const [keyboard, setKeyboard] = useState(false);
  const keys = useMemo(() => connections.map(quotaConnectionKey), [connections]);
  const indices = useMemo(() => new Map(keys.map((key, index) => [key, index])), [keys]);
  const focusIndex = indices.get(focusKey || activeKey) ?? 0;
  const select = useCallback((key: string) => {
    setFocusKey(key); setKeyboard(false); container.current?.focus({ preventScroll: true }); choose(key);
  }, [choose]);
  const { start, end } = quotaListWindow(connections.length, viewport.top, viewport.height);
  const visible = Array.from({ length: end - start }, (_, offset) => start + offset);
  // Keep the active descendant mounted when the user scrolls it out of view.
  if (connections.length && (focusIndex < start || focusIndex >= end)) visible.push(focusIndex);
  visible.sort((a, b) => a - b);

  useEffect(() => {
    const node = container.current;
    if (!node) return;
    const measure = () => setViewport({ top: node.scrollTop, height: node.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => { observer.disconnect(); cancelAnimationFrame(frame.current); };
  }, []);
  useEffect(() => {
    const node = container.current;
    if (node) { node.scrollTop = 0; setViewport({ top: 0, height: node.clientHeight }); }
    setKeyboard(false);
  }, [connections]);

  function navigate(event: KeyboardEvent<HTMLDivElement>) {
    if (!connections.length || event.altKey || event.ctrlKey || event.metaKey) return;
    let next = focusIndex;
    const page = Math.max(1, Math.floor(viewport.height / quotaRowHeight));
    if (event.key === 'ArrowDown') next++;
    else if (event.key === 'ArrowUp') next--;
    else if (event.key === 'PageDown') next += page;
    else if (event.key === 'PageUp') next -= page;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = connections.length - 1;
    else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); choose(keys[focusIndex]); return; }
    else return;
    event.preventDefault();
    next = Math.min(connections.length - 1, Math.max(0, next));
    setFocusKey(keys[next]); setKeyboard(true);
    const node = container.current;
    if (node) {
      const top = next * quotaRowHeight;
      if (top < node.scrollTop) node.scrollTop = top;
      else if (top + quotaRowHeight > node.scrollTop + node.clientHeight) node.scrollTop = top + quotaRowHeight - node.clientHeight;
      setViewport({ top: node.scrollTop, height: node.clientHeight });
    }
  }

  return <div className="quota-connection-list" ref={container} role="listbox" aria-label="平台连接"
    tabIndex={0} aria-activedescendant={connections.length ? `${id}-${focusIndex}` : undefined} onKeyDown={navigate}
    onScroll={() => {
      cancelAnimationFrame(frame.current);
      frame.current = requestAnimationFrame(() => {
        const node = container.current;
        if (node) setViewport({ top: node.scrollTop, height: node.clientHeight });
      });
    }}>
    <div className="quota-connection-items" style={{ height: connections.length * quotaRowHeight }}>
      {visible.map(index => <ConnectionRow key={keys[index]} id={`${id}-${index}`} index={index} count={connections.length} connection={connections[index]}
        selected={keys[index] === activeKey} focused={keyboard && index === focusIndex} choose={select}/>) }
    </div>
    {!connections.length ? <p className="meter-note" role="status">没有匹配的连接</p> : null}
  </div>;
});
