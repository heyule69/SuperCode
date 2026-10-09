export interface TrayChat { id: string; title: string; project: string; status: string; unread: boolean }
export interface TrayWindow { label: string; title: string; project: string; status: string; main: boolean }
export interface TraySnapshot { token: number; recent: TrayChat[]; hasMore: boolean; windows: TrayWindow[] }
export function trayStatus(status: string, unread = false) {
  if (status === 'starting' || status === 'running') return 'running';
  if (status === 'waiting') return 'waiting';
  return unread ? 'unread' : 'idle';
}
export function menuKeyIndex(key: string, current: number, count: number) {
  if (!count) return -1;
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  if (current < 0) return key === 'ArrowUp' ? count - 1 : 0;
  return (current + (key === 'ArrowUp' ? count - 1 : 1)) % count;
}
