import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { SessionIndicator } from './SessionIndicator';

describe('sidebar chat indicator', () => {
  it('shows activity ahead of an older unread result, including startup and approval waits', () => {
    for (const status of ['starting', 'running', 'waiting']) {
      const markup = renderToStaticMarkup(<SessionIndicator status={status} unread/>);
      expect(markup).toContain('session-spinner');
      expect(markup).not.toContain('unread-dot');
      expect(markup).toContain(status === 'waiting' ? '等待确认' : '运行中');
    }
  });
  it('shows the unread dot only for finished chats and removes it after reading', () => {
    for (const status of ['idle', 'completed', 'failed', 'interrupted']) {
      expect(renderToStaticMarkup(<SessionIndicator status={status} unread/>)).toContain('unread-dot');
      expect(renderToStaticMarkup(<SessionIndicator status={status} unread={false}/>)).toBe('');
    }
  });
});
