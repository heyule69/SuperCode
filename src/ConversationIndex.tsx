import { useLayoutEffect, useRef, useState } from 'react';
import { CornerDownRight, SquareTerminal, X } from 'lucide-react';
import type { ConversationTurn } from './events';
import { useFloatingLayer } from './useFloatingLayer';
import { scrollToConversationTurn } from './chatScroll';

export function indexedTurns(turns: ConversationTurn[]) {
  return turns.filter(turn => turn.user).map(turn => ({
    id: turn.id,
    question: turn.user!.text || '附件消息',
    reply: turn.items.filter(item => item.role === 'assistant' && item.text).at(-1)?.text.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').replace(/(\*\*|__)(.*?)\1/g, '$2').replace(/`+/g, '').replace(/(^|\n)\s*(?:[-*+] |#{1,6} )/g, '$1').replace(/\s+/g, ' ').slice(0, 220) || '',
    command: turn.items.some(item => item.kind === 'commandExecution' || item.kind === 'claudeToolCall' && item.data?.tool === 'Bash'),
  }));
}

type ScrollViewport = { top: number; height: number; scrollTop: number; scrollHeight: number };

export function visibleTurnIndex(positions: number[], viewport: ScrollViewport) {
  const first = positions.findIndex(Number.isFinite);
  if (first === -1 || viewport.height <= 0) return -1;
  let last = first;
  positions.forEach((top, index) => { if (Number.isFinite(top)) last = index; });
  // A short final round can remain below the reading point even at the bottom.
  if (viewport.scrollHeight - viewport.scrollTop - viewport.height <= 2 && positions[last] < viewport.top + viewport.height) return last;
  const readingPoint = viewport.top + viewport.height / 2;
  let active = first;
  positions.forEach((top, index) => { if (Number.isFinite(top) && top <= readingPoint) active = index; });
  return active;
}

export function ConversationIndex({ turns }: { turns: ConversationTurn[] }) {
  const items = indexedTurns(turns);
  const [active, setActive] = useState(items.at(-1)?.id);
  const [preview, setPreview] = useState<number | null>(null);
  const [bounds, setBounds] = useState({ left: 0, top: 0, height: 0, width: 320 });
  const layer = useFloatingLayer(preview != null, () => setPreview(null), { position: false });
  const navigation = useRef<(() => void) | null>(null);
  const target = preview == null ? undefined : items[preview];
  const signature = items.map(item => item.id).join(':');
  useLayoutEffect(() => {
    setPreview(null);
    const host = layer.root.current?.closest<HTMLElement>('.chat-scroll');
    if (!host) return;
    const nodes = new Map([...host.querySelectorAll<HTMLElement>('[data-conversation-turn]')].map(node => [node.dataset.conversationTurn, node]));
    const turnNodes = items.map(item => nodes.get(item.id));
    let frame = 0;
    const update = () => {
      frame = 0;
      const rect = host.getBoundingClientRect();
      if (!rect.height) return;
      const height = Math.min(items.length * 14, Math.max(42, rect.height - 80));
      const bounds = { left: rect.left + 8, top: rect.top + (rect.height - height) / 2, height, width: Math.max(160, Math.min(340, rect.width - 52)) };
      setBounds(previous => previous.left === bounds.left && previous.top === bounds.top && previous.height === bounds.height && previous.width === bounds.width ? previous : bounds);
      const index = visibleTurnIndex(turnNodes.map(node => node?.getBoundingClientRect().top ?? Infinity), { top: rect.top, height: host.clientHeight, scrollTop: host.scrollTop, scrollHeight: host.scrollHeight });
      setActive(items[index]?.id);
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };
    update(); schedule();
    const observer = new ResizeObserver(schedule);
    observer.observe(host);
    const content = layer.root.current?.closest('.messages');
    if (content) observer.observe(content);
    turnNodes.forEach(node => { if (node) observer.observe(node); });
    host.addEventListener('scroll', schedule, { passive: true });
    window.addEventListener('resize', schedule);
    return () => { navigation.current?.(); navigation.current = null; observer.disconnect(); host.removeEventListener('scroll', schedule); window.removeEventListener('resize', schedule); if (frame) cancelAnimationFrame(frame); };
  }, [signature]);
  function jump(index: number) {
    const host = layer.root.current?.closest<HTMLElement>('.chat-scroll');
    const node = [...(host?.querySelectorAll<HTMLElement>('[data-conversation-turn]') ?? [])].find(el => el.dataset.conversationTurn === items[index]?.id);
    setPreview(null);
    navigation.current?.();
    navigation.current = null;
    if (!host || !node) return;
    navigation.current = scrollToConversationTurn(host, node, document.documentElement.dataset.motion === 'off' || window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth');
  }
  if (!items.length) return null;
  return <div ref={layer.root} className="conversation-index" style={{ left: bounds.left, top: bounds.top, height: bounds.height || items.length * 14 }}>
    <nav aria-label="对话轮次" onKeyDown={event => {
      if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const buttons = [...(layer.root.current?.querySelectorAll<HTMLButtonElement>('nav button') ?? [])];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowDown' ? (index + 1) % buttons.length : (index <= 0 ? buttons.length : index) - 1]?.focus();
    }}>{items.map((item, index) => <button key={item.id} type="button" aria-label={`查看第 ${index + 1} 轮对话`} aria-current={active === item.id ? 'true' : undefined} aria-expanded={preview === index} onFocus={() => setPreview(index)} onMouseEnter={() => setPreview(index)} onClick={() => jump(index)}><span aria-hidden="true" /></button>)}</nav>
    {target ? <section className="conversation-peek" role="dialog" aria-label={`第 ${preview! + 1} 轮预览`} style={{ width: bounds.width, top: Math.max(0, Math.min(preview! * 14 - 35, bounds.height - 160)) }}>
      <div className="conversation-peek-heading"><strong>{target.question}</strong><button type="button" className="icon-button" aria-label="关闭轮次预览" onClick={() => { setPreview(null); layer.root.current?.querySelectorAll<HTMLButtonElement>('nav button')[preview!]?.blur(); }}><X size={13}/></button></div>
      {target.reply ? <p>{target.reply}</p> : <p>尚无回复</p>}
      <div className="conversation-peek-footer">{target.command ? <span><SquareTerminal size={13}/>运行了命令</span> : <span>第 {preview! + 1} 轮</span>}<button type="button" onClick={() => jump(preview!)}><CornerDownRight size={13}/>定位到本轮</button></div>
    </section> : null}
  </div>;
}
