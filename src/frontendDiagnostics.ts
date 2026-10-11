import { invoke, isTauri } from '@tauri-apps/api/core';

export function reportFrontendError(source: string, error: unknown, command?: string, componentStack?: string) {
  if (!isTauri() || command === 'report_frontend_error') return;
  const message = error instanceof Error ? error.message : String(error);
  const stack = [error instanceof Error ? error.stack : '', componentStack].filter(Boolean).join('\n');
  // Send no IPC arguments or conversation state. The native logger redacts before writing.
  void invoke('report_frontend_error', { source, message, stack: stack || null, command: command ?? null }).catch(() => {});
}

export function installFrontendDiagnostics() {
  const onError = (event: ErrorEvent) => reportFrontendError('javascript', event.error ?? event.message);
  const onRejection = (event: PromiseRejectionEvent) => reportFrontendError('promise', event.reason);
  window.addEventListener('error', onError);
  window.addEventListener('unhandledrejection', onRejection);
  return () => {
    window.removeEventListener('error', onError);
    window.removeEventListener('unhandledrejection', onRejection);
  };
}
