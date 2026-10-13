import { useCallback, useRef } from 'react';

/** Stable event identity with the current render's permissions, route and draft. */
export function useStableCallback<T extends (...args: any[]) => any>(callback: T): T {
  const latest = useRef(callback); latest.current = callback;
  return useCallback(((...args: Parameters<T>) => latest.current(...args)) as T, []);
}
