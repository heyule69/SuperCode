export interface ExtensionPlugin {
  id: string; nativeId: string; agent: string; name: string; description: string;
  version: string; path: string; installed: boolean; enabled: boolean; source: string;
  components: string[]; icon: string | null; limitation: string | null;
}
export interface ExtensionMcp {
  id: string; name: string; agent: string; source: string; endpoint: string;
  transport: string; enabled: boolean; pluginId: string | null; limitation: string | null;
}
export interface ExtensionCatalog { plugins: ExtensionPlugin[]; mcp: ExtensionMcp[]; warnings: string[] }
export interface ToolServer { id: string; name: string; kind: string; command: string; args: string[]; url: string | null; enabled: boolean }
export function matchesExtension(item: { name: string; agent: string; source: string; description?: string }, agent: string, search: string) {
  return (agent === 'all' || item.agent === agent || item.agent === 'both') && `${item.name} ${item.source} ${item.description ?? ''}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase());
}
export function pluginParentEnabled(mcp: ExtensionMcp, plugins: ExtensionPlugin[]) {
  return !mcp.pluginId || plugins.some(p => p.id === mcp.pluginId && p.enabled);
}
export function parseToolArgs(text: string): string[] {
  const args: unknown = text.trim().startsWith('[') ? JSON.parse(text) : text.split('\n').map(v => v.replace(/\r$/, '')).filter(Boolean);
  if (!Array.isArray(args) || args.some(v => typeof v !== 'string')) throw new Error('参数必须为字符串数组');
  return args;
}
