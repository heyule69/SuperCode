import { LoaderCircle } from 'lucide-react';
import { isActive } from './types';

export function SessionIndicator({ status, unread }: { status: string; unread?: boolean }) {
  if (isActive(status)) return <LoaderCircle size={14} className="session-indicator session-spinner spin" aria-label={status === 'waiting' ? '等待确认' : '运行中'}/>;
  if (unread) return <span className="session-indicator unread-dot" aria-label="未读"/>;
  return null;
}
