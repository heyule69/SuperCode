export type AutomationKind = 'browser' | 'computer';
export interface AutomationStatus {
  kind: AutomationKind;
  phase: 'notInstalled' | 'missing' | 'preparing' | 'downloading' | 'installing' | 'testing' | 'ready' | 'failed' | 'cancelled';
  message: string;
  received: number | null;
  total: number | null;
  enabled: boolean;
  installation: { version: string; tools: number; testedAt: number; summary: string; root: string } | null;
}
export const automationKinds: AutomationKind[] = ['browser', 'computer'];
export function automationWorking(phase: AutomationStatus['phase']) { return ['preparing', 'downloading', 'installing', 'testing'].includes(phase); }
export function automationDownload(status: Pick<AutomationStatus, 'received' | 'total'>) {
  if (status.received == null) return '';
  const size = `${(status.received / 1048576).toFixed(1)} MB`;
  return status.total && status.total >= status.received ? `${size} / ${(status.total / 1048576).toFixed(1)} MB` : size;
}
export function mergeAutomationProgress(items: AutomationStatus[], next: AutomationStatus) {
  // Progress events only carry live state; preserve the previous successful installation.
  return items.map(item => item.kind === next.kind ? { ...next, installation: next.installation ?? item.installation, enabled: next.installation ? next.enabled : item.enabled } : item);
}
