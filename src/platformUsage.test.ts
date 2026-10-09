import { describe, expect, it, vi } from 'vitest';
import { allowanceSummary, resetLabel, UsageQueries, type PlatformUsage } from './platformUsage';
const example: PlatformUsage = { agent: 'claude', connectionId: 'a', providerName: 'GLM', connectionName: 'GLM Coding', queriedAt: 1, windows: [], balances: [] };
describe('platform allowance display', () => {
  it('shows the tightest real window, including exhausted quota', () => {
    expect(allowanceSummary({ ...example, windows: [{ label: '5h', remainingPercent: 80 }, { label: 'week', remainingPercent: 0 }] })).toBe('剩余 0%');
    expect(allowanceSummary(example)).toBe('');
  });
  it('keeps key budget distinct from the wallet and accepts a real zero balance', () => {
    expect(allowanceSummary({ ...example, balances: [{ label: 'Key 预算余额', amount: 0, currency: 'USD' }] })).toBe('Key 预算余额 $0.00');
    expect(resetLabel('invalid')).toBe(''); expect(resetLabel(null)).toBe('');
    expect(resetLabel('1800000000')).toContain('重置');
  });
});

describe('bounded quota queries', () => {
  it('shares concurrent requests and serves revisited connections from memory', async () => {
    let time = 0;
    const read = vi.fn(async () => example);
    const queries = new UsageQueries(read, () => time);
    const first = queries.query('claude', 'a');
    expect(queries.query('claude', 'a')).toBe(first);
    await first; await queries.query('claude', 'a');
    expect(read).toHaveBeenCalledTimes(1);
    time = 60_001; await queries.query('claude', 'a');
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('retains a known value while refreshing, and never turns unavailable data into zero', async () => {
    let reply: PlatformUsage | null = example;
    const read = vi.fn(async () => reply);
    const queries = new UsageQueries(read);
    await queries.query('claude', 'a');
    reply = null;
    const update = queries.query('claude', 'a', true);
    expect(queries.peek('claude', 'a')).toBe(example);
    await update;
    expect(queries.peek('claude', 'a')).toBeNull();
    await queries.query('claude', 'a');
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('shortly caches unavailable responses but retries them after 20 seconds', async () => {
    let time = 0;
    const read = vi.fn(async () => null);
    const queries = new UsageQueries(read, () => time);
    await queries.query('claude', 'a'); await queries.query('claude', 'a');
    expect(read).toHaveBeenCalledTimes(1);
    time = 20_001; await queries.query('claude', 'a');
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('serializes a manual refresh behind an initial request instead of racing it', async () => {
    let complete!: (value: PlatformUsage) => void;
    const read = vi.fn((_agent: string, _id: string, force: boolean) => force ? Promise.resolve({ ...example, queriedAt: 2 }) : new Promise<PlatformUsage>(resolve => { complete = resolve; }));
    const queries = new UsageQueries(read);
    const first = queries.query('claude', 'a');
    const refresh = queries.query('claude', 'a', true);
    await Promise.resolve(); expect(read).toHaveBeenCalledTimes(1);
    complete(example); await first; expect((await refresh)?.queriedAt).toBe(2);
    expect(read.mock.calls.map(call => call[2])).toEqual([false, true]);
  });
  it('isolates connection revisions and prevents late old responses overwriting a newer query', async () => {
    let complete!: (value: PlatformUsage) => void;
    const read = vi.fn().mockImplementationOnce(() => new Promise<PlatformUsage>(resolve => { complete = resolve; })).mockResolvedValue({ ...example, queriedAt: 2 });
    const queries = new UsageQueries(read);
    const first = queries.query('claude', 'a', false, 'old'); await Promise.resolve();
    const force = queries.query('claude', 'a', true, 'old');
    await queries.query('claude', 'a', false, 'new');
    complete(example); await first; await force;
    expect(queries.peek('claude', 'a', 'old')).toBeUndefined();
    expect(queries.peek('claude', 'a', 'new')?.queriedAt).toBe(2);
    expect(read).toHaveBeenCalledTimes(2);
  });
  it('limits the cache to 64 recently visited connections and drops expired entries', async () => {
    let time = 0;
    const queries = new UsageQueries(async () => example, () => time);
    for (let index = 0; index < 64; index++) await queries.query('claude', String(index));
    queries.peek('claude', '0'); await queries.query('claude', '64');
    expect(queries.peek('claude', '1')).toBeUndefined(); expect(queries.peek('claude', '0')).toBe(example);
    time = 300_000; expect(queries.peek('claude', '0')).toBeUndefined();
  });
});
