import { useEffect, useRef, useState } from 'react';
import { call } from './api';
import { isActive } from './types';

interface ReadReceipt {
  sessionId: string;
  status?: string;
  unread: boolean;
  throughSeq: number;
  loading: boolean;
  settingsOpen: boolean;
  onRead: (sessionId: string) => void;
  onError: (error: unknown) => void;
}
const visible = () => document.visibilityState === 'visible' && document.hasFocus();

export function useChatReadReceipt({ sessionId, status, unread, throughSeq, loading, settingsOpen, onRead, onError }: ReadReceipt) {
  const [viewing, setViewing] = useState(visible);
  const latest = useRef({ unread, onRead, onError });
  latest.current = { unread, onRead, onError };
  useEffect(() => {
    const update = () => setViewing(visible());
    window.addEventListener('focus', update);
    window.addEventListener('blur', update);
    document.addEventListener('visibilitychange', update);
    return () => {
      window.removeEventListener('focus', update);
      window.removeEventListener('blur', update);
      document.removeEventListener('visibilitychange', update);
    };
  }, []);
  useEffect(() => {
    if (!sessionId || !status || loading || settingsOpen || !viewing || isActive(status) || !latest.current.unread) return;
    let disposed = false;
    void call<boolean>('mark_session_read', { sessionId, throughSeq }).then(changed => {
      if (!disposed && changed) latest.current.onRead(sessionId);
    }).catch(error => { if (!disposed) latest.current.onError(error); });
    return () => { disposed = true; };
    // Read on opening, loading new results or returning to the chat. Changing only
    // the manual unread flag must not immediately undo "标记为未读".
  }, [sessionId, status, throughSeq, loading, settingsOpen, viewing]);
}
