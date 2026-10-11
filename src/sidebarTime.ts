export function sidebarTime(updatedAt: number, now = Date.now()): string {
  const milliseconds = updatedAt * 1000;
  if (!Number.isFinite(milliseconds) || milliseconds <= 0) return '';
  const minutes = Math.max(0, Math.floor((now - milliseconds) / 60000));
  if (minutes < 1) return '刚刚';
  if (minutes < 60) return `${minutes}分钟前`;
  if (minutes < 1440) return `${Math.floor(minutes / 60)}小时前`;
  if (minutes < 10080) return `${Math.floor(minutes / 1440)}天前`;
  return new Date(milliseconds).toLocaleDateString('zh-CN', { month: 'numeric', day: 'numeric' });
}
