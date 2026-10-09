import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { QuotaConnections, quotaConnectionKey, quotaListWindow, quotaRowHeight } from './QuotaConnections';
import type { QuotaConnection } from './platformUsage';

describe('large quota connection lists', () => {
  it('renders only a viewport and buffer for 5,000 connections, retaining the focused descendant', () => {
    const connections: QuotaConnection[] = Array.from({ length: 5000 }, (_, index) => ({ connectionId: `c${index}`, agent: 'claude', connectionName: `连接 ${index}`, source: { providerId: 'custom', providerName: 'API', mark: '连' } }));
    const html = renderToStaticMarkup(<QuotaConnections connections={connections} activeKey={quotaConnectionKey(connections[4999])} choose={() => {}}/>);
    expect((html.match(/role="option"/g) ?? []).length).toBeLessThan(25);
    expect(html).toContain('连接 4999');
    expect(html).toContain('aria-setsize="5000"');
    expect(html).toContain('aria-posinset="5000"');
    expect(html).toContain('aria-selected="true"');
  });
  it('bounds the window near the middle and end and handles empty or shrinking lists', () => {
    const middle = quotaListWindow(5000, 2400 * quotaRowHeight, 600);
    expect(middle.start).toBe(2396); expect(middle.end - middle.start).toBeLessThan(20);
    expect(quotaListWindow(5000, 4999 * quotaRowHeight, 600).end).toBe(5000);
    expect(quotaListWindow(0, 5000, 600)).toEqual({ start: 0, end: 0 });
    expect(quotaListWindow(3, 5000, 600)).toEqual({ start: 2, end: 3 });
  });
  it('keeps same-named connections and shared local identifiers distinct across engines', () => {
    const source = { providerId: 'custom', providerName: 'API', mark: 'A' };
    const claude = { agent: 'claude', connectionId: '@local', connectionName: 'API', source };
    expect(quotaConnectionKey(claude)).not.toBe(quotaConnectionKey({ ...claude, agent: 'pi' }));
  });
});
