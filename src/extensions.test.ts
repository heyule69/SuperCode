import { describe, expect, it } from 'vitest';
import { matchesExtension, parseToolArgs, pluginParentEnabled, type ExtensionMcp, type ExtensionPlugin } from './extensions';
describe('extension inventory controls', () => {
  it('filters by agent and searches names and metadata without hiding shared MCPs', () => {
    expect(matchesExtension({ name: 'Context7', source: '本机', agent: 'claude' }, 'codex', '')).toBe(false);
    expect(matchesExtension({ name: 'Playwright', source: 'SuperCode', agent: 'both' }, 'claude', ' supercode ')).toBe(true);
    expect(matchesExtension({ name: 'PDF', source: 'OpenAI', agent: 'codex', description: 'Read documents' }, 'all', 'DOCUMENT')).toBe(true);
  });
  it('does not enable bundled MCPs while their plugin is disabled or missing', () => {
    const mcp = { pluginId: 'p' } as ExtensionMcp;
    expect(pluginParentEnabled(mcp, [])).toBe(false);
    expect(pluginParentEnabled(mcp, [{ id: 'p', enabled: false } as ExtensionPlugin])).toBe(false);
    expect(pluginParentEnabled(mcp, [{ id: 'p', enabled: true } as ExtensionPlugin])).toBe(true);
    expect(pluginParentEnabled({ pluginId: null } as ExtensionMcp, [])).toBe(true);
  });
  it('keeps arguments containing spaces and shell syntax as separate literal arguments', () => {
    expect(parseToolArgs('C:\\a b\\server.js\n$(literal)\n\n--flag')).toEqual(['C:\\a b\\server.js', '$(literal)', '--flag']);
    expect(parseToolArgs('["a b", "--test"]')).toEqual(['a b', '--test']);
    expect(() => parseToolArgs('["a", 1]')).toThrow();
    expect(() => parseToolArgs('[bad]')).toThrow();
  });
});
