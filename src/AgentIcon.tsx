import { useState } from 'react';
import brands from '../resources/provider-brands.json';

export function AgentIcon({ agent }: { agent: string }) {
  if (agent === 'claude') return <img className="agent-logo" src="/brands/claude.png" alt="" aria-hidden="true"/>;
  if (agent === 'opencode' || agent === 'pi') return <img className="agent-logo" src={`/brands/${agent}.svg`} alt="" aria-hidden="true"/>;
  if (agent !== 'codex') return <span className="agent-logo" aria-hidden="true">{agent.slice(0, 1).toUpperCase()}</span>;
  return <span className="agent-logo codex-logo" aria-hidden="true"><img className="codex-logo-light" src="/brands/codex-light.png" alt=""/><img className="codex-logo-dark" src="/brands/codex-dark.png" alt=""/></span>;
}
export function providerInitial(name = '', mark = '') {
  return Array.from(name.trim())[0]?.toLocaleUpperCase() || (/^[\p{L}\p{N}]/u.test(mark) ? Array.from(mark)[0].toLocaleUpperCase() : '?');
}
export function ProviderIcon({ provider, mark, name }: { provider: string; mark?: string; name?: string }) {
  const [failed, setFailed] = useState('');
  if (provider === 'anthropic' || provider === 'openai') return <AgentIcon agent={provider === 'anthropic' ? 'claude' : 'codex'}/>;
  const source = (brands as Record<string, string>)[provider];
  return source && failed !== source ? <img className="provider-logo" src={source} alt="" aria-hidden="true" onError={() => setFailed(source)}/> : <span className="provider-initial" aria-hidden="true">{providerInitial(name, mark)}</span>;
}
