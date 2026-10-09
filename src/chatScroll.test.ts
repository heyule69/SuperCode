import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { scrollToConversationTurn } from './chatScroll';

function fixture() {
  const state = { top: 3000, start: 900, connected: true };
  const scrollTo = vi.fn(({ top }: ScrollToOptions) => { state.top = top!; });
  const area = Object.assign(new EventTarget(), { clientHeight: 500, scrollHeight: 5000, getBoundingClientRect: () => ({ top: 100 }), scrollTo });
  Object.defineProperty(area, 'scrollTop', { get: () => state.top });
  const item = { getBoundingClientRect: () => ({ top: 100 + state.start - state.top, height: 400 }), get isConnected() { return state.connected; } };
  return { state, scrollTo, area: area as unknown as HTMLElement, item: item as unknown as HTMLElement };
}

describe('conversation scroll destination', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['performance', 'setTimeout', 'clearTimeout'] });
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => setTimeout(() => callback(performance.now()), 16));
    vi.stubGlobal('cancelAnimationFrame', (id: ReturnType<typeof setTimeout>) => clearTimeout(id));
  });
  afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });
  it('reaches the selected question when earlier content grows during navigation', () => {
    const { area, item, state, scrollTo } = fixture();
    scrollToConversationTurn(area, item, 'smooth');
    setTimeout(() => { state.start += 400; }, 40);
    vi.advanceTimersByTime(800);
    expect(item.getBoundingClientRect().top).toBe(310);
    expect(scrollTo).toHaveBeenCalledTimes(2);
    expect(scrollTo.mock.calls.every(([options]) => options.behavior === 'smooth')).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('does not pull the user back after they scroll manually', () => {
    const { area, item, state, scrollTo } = fixture();
    scrollToConversationTurn(area, item, 'smooth');
    vi.advanceTimersByTime(48);
    area.dispatchEvent(new Event('wheel'));
    state.top = 1400;
    state.start += 400;
    vi.advanceTimersByTime(800);
    expect(state.top).toBe(1400);
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('cancels corrections when another round is selected or the component unmounts', () => {
    const { area, item, state, scrollTo } = fixture();
    const stop = scrollToConversationTurn(area, item, 'smooth');
    stop();
    state.start += 400;
    vi.advanceTimersByTime(800);
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('stops when the target is removed', () => {
    const { area, item, state, scrollTo } = fixture();
    scrollToConversationTurn(area, item, 'smooth');
    state.connected = false;
    state.start += 400;
    vi.advanceTimersByTime(800);
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
  it('bounds corrections for continuously changing history', () => {
    const { area, item, state, scrollTo } = fixture();
    scrollToConversationTurn(area, item, 'instant');
    for (let i = 0; i < 20; i++) { state.start += 60; vi.advanceTimersByTime(16); }
    expect(scrollTo.mock.calls.length).toBeLessThanOrEqual(4);
    expect(vi.getTimerCount()).toBe(0);
  });
});
