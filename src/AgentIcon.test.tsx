import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ProviderIcon, providerInitial } from './AgentIcon';
import brands from '../resources/provider-brands.json';
import catalog from '../resources/providers.json';

describe('local provider identities', () => {
  it('uses local brand assets for platform presets and initials for custom connections', () => {
    expect(renderToStaticMarkup(<ProviderIcon provider="kimi" name="my connection"/>)).toContain('/brands/kimi.png');
    const custom = renderToStaticMarkup(<ProviderIcon provider="custom" name="Kimi proxy"/>);
    expect(custom).toContain('>K</span>'); expect(custom).not.toContain('/brands/');
    expect(providerInitial('我的连接')).toBe('我');
    expect(providerInitial('  local gateway')).toBe('L');
  });
  it('covers every bundled official provider without external image requests', () => {
    for (const provider of catalog.filter(p => p.id !== 'custom')) {
      const html = renderToStaticMarkup(<ProviderIcon provider={provider.id} name={provider.name}/>);
      expect(html, provider.id).toContain('/brands/');
      expect(html).not.toContain('src="https:');
    }
    expect(Object.values(brands).every(src => src.startsWith('/brands/'))).toBe(true);
  });
});
