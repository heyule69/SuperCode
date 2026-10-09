import { describe, expect, it } from 'vitest';
import { automationDownload, automationWorking, mergeAutomationProgress, type AutomationStatus } from './automation';
describe('automation installation state', () => {
  it('preserves a working installation while a repair is in progress or fails', () => {
    const ready = { kind: 'browser', phase: 'ready', enabled: true, installation: { version: '1', tools: 4, testedAt: 123, summary: 'passed', root: 'private' } } as AutomationStatus;
    const other = { kind: 'computer', phase: 'notInstalled' } as AutomationStatus;
    const result = mergeAutomationProgress([ready, other], { kind: 'browser', phase: 'failed', message: 'network failure', installation: null } as AutomationStatus);
    expect(result[0].installation).toEqual(ready.installation);
    expect(result[0].enabled).toBe(true);
    expect(result[0].phase).toBe('failed');
    expect(result[1]).toBe(other);
  });
  it('keeps actions locked only during a real operation', () => {
    expect(automationWorking('testing')).toBe(true);
    expect(automationWorking('downloading')).toBe(true);
    expect(automationWorking('failed')).toBe(false);
    expect(automationWorking('ready')).toBe(false);
  });
  it('does not invent download totals when the response has no content length', () => {
    expect(automationDownload({received: 1048576, total: null})).toBe('1.0 MB');
    expect(automationDownload({received: null, total: null})).toBe('');
  });
});
