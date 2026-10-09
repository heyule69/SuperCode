export type Destination = { page: 'chat'; projectId: string; sessionId: string } | { page: 'settings'; tab: string };
export interface Navigation { entries: Destination[]; index: number }
export const emptyNavigation: Navigation = { entries: [], index: -1 };
export function visit(state: Navigation, target: Destination): Navigation {
  if (JSON.stringify(state.entries[state.index]) === JSON.stringify(target)) return state;
  if (state.entries[state.index]?.page === 'settings') {
    if (target.page === 'settings') {
      const entries = [...state.entries]; entries[state.index] = target;
      return { ...state, entries };
    }
    const previous = state.entries[state.index - 1];
    if (JSON.stringify(previous) === JSON.stringify(target)) return { ...state, index: state.index - 1 };
  }
  const entries = [...state.entries.slice(0, state.index + 1), target].slice(-40);
  return { entries, index: entries.length - 1 };
}
export function travel(state: Navigation, direction: -1 | 1): Navigation {
  const index = state.index + direction;
  return index < 0 || index >= state.entries.length ? state : { ...state, index };
}
