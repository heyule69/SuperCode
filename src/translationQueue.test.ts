import { expect, it, vi } from 'vitest';
import { createTranslationQueue, type TranslationState } from './translationQueue';
const tick = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

it('serializes, deduplicates, caches and isolates sessions without translating abandoned queued work', async () => {
  const pending: ((text: string) => void)[] = [];
  const request = vi.fn(() => new Promise<string>(resolve => pending.push(resolve)));
  const queue = createTranslationQueue(request);
  const first = vi.fn(), duplicate = vi.fn(), second = vi.fn(), abandoned = vi.fn();
  queue.watch('a', 'First sentence.', first);
  queue.watch('a', 'First sentence.', duplicate);
  queue.watch('b', 'First sentence.', second);
  const release = queue.watch('a', 'Never translated.', abandoned);
  release();
  expect(request).toHaveBeenCalledTimes(1);
  pending.shift()!('第一句话。'); await tick();
  expect(first).toHaveBeenLastCalledWith({ status:'ready', text:'第一句话。' });
  expect(duplicate).toHaveBeenLastCalledWith({ status:'ready', text:'第一句话。' });
  expect(request).toHaveBeenCalledTimes(2);
  expect(request).toHaveBeenLastCalledWith('b', 'First sentence.');
  pending.shift()!('另一个聊天的译文。'); await tick();
  expect(request).toHaveBeenCalledTimes(2);
  const cached = vi.fn(); queue.watch('a', 'First sentence.', cached);
  expect(cached).toHaveBeenLastCalledWith({ status:'ready', text:'第一句话。' });
  expect(request).toHaveBeenCalledTimes(2);
});

it('keeps failed requests quiet until explicit retry and validates before reuse', async () => {
  const request = vi.fn().mockResolvedValueOnce('损坏的译文').mockResolvedValueOnce('请阅读 ⟪SC0⟫。');
  const queue = createTranslationQueue(request), listener = vi.fn();
  queue.watch('a', 'Read ⟪SC0⟫.', listener); await tick();
  expect(listener.mock.lastCall?.[0].status).toBe('error');
  expect(request).toHaveBeenCalledTimes(1);
  queue.watch('a', 'Read ⟪SC0⟫.', () => {}); await tick();
  expect(request).toHaveBeenCalledTimes(1);
  queue.retry('a', 'Read ⟪SC0⟫.'); await tick();
  expect(listener).toHaveBeenLastCalledWith({ status:'ready', text:'请阅读 ⟪SC0⟫。' });
});

it('bounds outstanding work and drops listeners after unmount', async () => {
  let finish!: (value: string) => void;
  const request = vi.fn(() => new Promise<string>(resolve => { finish = resolve; }));
  const queue = createTranslationQueue(request);
  const listener = vi.fn(); const release = queue.watch('a', 'Message 0.', listener);
  for (let i = 1; i < 48; i++) queue.watch('a', `Message ${i}.`, () => {});
  let overflow: TranslationState | undefined;
  queue.watch('a', 'Too much queued.', state => { overflow = state; });
  expect(overflow?.status).toBe('error');
  release(); listener.mockClear(); finish('已完成。'); await tick();
  expect(listener).not.toHaveBeenCalled();
  expect(request).toHaveBeenCalledTimes(2);
});
