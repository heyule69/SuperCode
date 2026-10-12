import { createContext } from 'react';

export const PlanDisplayContext = createContext<{
  openPlan?: (name: string, text: string, openFile?: (path: string, line?: number) => void) => void;
  decisionContainer?: HTMLElement | null;
}>({});

export function planTitle(text: string) {
  const heading = text.match(/^#{1,2}\s+(.+)$/m)?.[1];
  return heading?.replace(/[*_`]/g, '').trim() || '计划';
}
