import { describe, expect, it, vi } from 'vitest';
import { allowanceSummary, primaryAllowance, resetLabel, supportsPlatformUsage, UsageQueries, type PlatformUsage } from './platformUsage';
const example: PlatformUsage = { agent: 'claude', connectionId: 'a', providerName: 'GLM', connectionName: 'GLM Coding', queriedAt: 1, windows: [], balances: [] };
describe('platform allowance display', () => {
  it('prioritizes the short model window over weekly and MCP quota, with matching meter data', () => {
    const value = { ...example, windows: [{ label: '周额度', remainingPercent: 87, windowDurationMins: 10080 }, { label: 'MCP 额度', remainingPercent: 1, scope: 'tool' as const }, { label: '5 小时额度', remainingPercent: 97, windowDurationMins: 300 }] };
    expect(allowanceSummary(value)).toBe('5 小时额度 · 剩余 97%');
    expect(primaryAllowance(value)?.remainingPercent).toBe(97);
    expect(allowanceSummary(example)).toBe('');
  });
  it('uses actual provider durations and retains real zero short-window allowance', () => {
    const value = { ...example, windows: [{ label: '日额度', remainingPercent: 12, windowDurationMins: 1440 }, { label: '2 小时额度', remainingPercent: 0, windowDurationMins: 120 }] };
    expect(allowanceSummary(value)).toBe('2 小时额度 · 剩余 0%');
    expect(allowanceSummary({ ...example, windows: [{ label: '周额度', remainingPercent: 35 }] })).toBe('周额度 · 剩余 35%');
    expect(allowanceSummary({ ...example, windows: [{ label: '套餐额度', remainingPercent: 50 }] })).toBe('套餐额度 · 剩余 50%');
  });
  it('handles old cached labels and does not invent quota for invalid or balance-only responses', () => {
    expect(allowanceSummary({ ...example, windows: [{ label: 'week', remainingPercent: 0 }, { label: '5h', remainingPercent: 80 }] })).toBe('5h · 剩余 80%');
    expect(primaryAllowance({ ...example, windows: [{ label: '5 小时额度', remainingPercent: NaN }, { label: '周额度', remainingPercent: -1 }] })).toBeUndefined();
    expect(primaryAllowance({ ...example, balances: [{ label: 'API 余额', amount: 5, currency: 'CNY' }] })).toBeUndefined();
  });
  it('keeps key budget distinct from the wallet and accepts a real zero balance', () => {
    expect(allowanceSummary({ ...example, balances: [{ label: 'Key 预算余额', amount: 0, currency: 'USD' }] })).toBe('Key 预算余额 $0.00');
    expect(resetLabel('invalid')).toBe(''); expect(resetLabel(null)).toBe('');
    expect(resetLabel('1800000000')).toContain('重置');
  });
});

describe('bounded quota queries', () => {
  it('retains a separately marked last successful value after an unavailable update and expires it independently',async()=>{
    let time=0;
    const read=vi.fn().mockResolvedValueOnce(example).mockResolvedValue(null);
    const queries=new UsageQueries(read,()=>time);
    await queries.query('claude','a',false,'revision');time=10_000;
    await queries.query('claude','a',true,'revision');
    expect(queries.peek('claude','a','revision')).toBeNull();
    expect(queries.lastKnown('claude','a','revision')).toBe(example);
    expect(queries.lastKnown('claude','other','revision')).toBeUndefined();
    expect(queries.lastKnown('claude','a','changed-key')).toBeUndefined();
    time=300_001;await queries.query('claude','a',true,'revision');
    expect(queries.lastKnown('claude','a','revision')).toBeUndefined();
  });
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
it('only reserves a quota entry for supported providers when no data has been returned',()=>{
  expect(supportsPlatformUsage('claude',{providerId:'glm',providerName:'GLM',mark:'Z'})).toBe(true);
  expect(supportsPlatformUsage('pi',{providerId:'kimi',providerName:'Kimi',mark:'K'})).toBe(true);
  expect(supportsPlatformUsage('claude',{providerId:'custom',providerName:'Local',mark:'L'})).toBe(false);
});
