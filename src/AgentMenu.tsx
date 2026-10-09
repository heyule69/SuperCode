import { useEffect, useState } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import type { Agent } from './types';
import { useFloatingLayer } from './useFloatingLayer';
import { AgentIcon } from './AgentIcon';

export function AgentMenu({ value, agents, busy, choose, label = 'Agent' }: { value: string; agents: Agent[]; busy: boolean; choose: (id: string) => void; label?: string }) {
  const [open, setOpen] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false), { focusFirst: true });
  useEffect(() => { if (busy) setOpen(false); }, [busy]);
  return <div className="composer-menu-anchor agent-menu" ref={layer.root} onKeyDown={layer.navigate}>
    <button type="button" className="composer-chip agent-chip" aria-label={label} aria-haspopup="menu" aria-expanded={open} disabled={busy} onClick={() => setOpen(v => !v)} onKeyDown={event => { if (event.key === 'ArrowDown') { event.preventDefault(); setOpen(true); } }}><AgentIcon agent={value}/><span>{agents.find(agent => agent.id === value)?.name ?? value}</span><ChevronDown size={11}/></button>
    {open ? <div className="composer-popover agent-popover" role="menu" aria-label="选择 Agent">{agents.filter(agent => agent.connected).map(agent => <button type="button" key={agent.id} role="menuitemradio" aria-checked={agent.id === value} onClick={() => { choose(agent.id); layer.dismiss(true); }}><AgentIcon agent={agent.id}/><span><strong>{agent.name}</strong><small>{({ claude: 'Claude Code 与兼容连接', codex: 'Codex 与本机账号', opencode: 'OpenCode 与本机模型', pi: 'Pi 与本机模型' } as Record<string, string>)[agent.id]}</small></span><span className="agent-option-check">{agent.id === value ? <Check size={14}/> : null}</span></button>)}</div> : null}
  </div>;
}
