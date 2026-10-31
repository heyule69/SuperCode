import { useEffect, useRef } from 'react';
import { getCurrentWebview, type DragDropEvent } from '@tauri-apps/api/webview';
import type { Event as TauriEvent, UnlistenFn } from '@tauri-apps/api/event';
import { desktop } from './api';
import { useStableCallback } from './useStableCallback';

export function useComposerFileDrop(addPaths: (paths: string[]) => Promise<void>, report: (error: unknown) => void, enabled: boolean) {
  const root = useRef<HTMLFormElement>(null);
  const reportError = useStableCallback(report);
  const drop = useStableCallback(({ payload }: TauriEvent<DragDropEvent>) => {
    if (!enabled || payload.type !== 'drop' || !payload.paths.length || !root.current) return;
    // Tauri reports physical pixels; DOM hit testing uses CSS pixels.
    const target = document.elementFromPoint(payload.position.x / window.devicePixelRatio, payload.position.y / window.devicePixelRatio);
    if (!target || !root.current.contains(target) || target.closest('[inert], [hidden]')) return;
    void addPaths(payload.paths).catch(reportError);
  });
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void getCurrentWebview().onDragDropEvent(event => { if (!disposed) drop(event); }).then(stop => {
      if (disposed) stop(); else unlisten = stop;
    }).catch(reportError);
    return () => { disposed = true; unlisten?.(); };
  }, [drop, reportError]);
  return root;
}
