export interface Project { id: string; name: string; path: string }
export interface Session { id: string; projectId: string; workspacePath?: string | null; title: string; agent: string; model: string | null; connectionId?: string | null; nativeId: string | null; status: string; updatedAt: number; turnId: string | null }
export interface Message { seq: number; id: string; sessionId: string; role: string; text: string; kind: string; data: Record<string, unknown> | null }
export interface Agent { id: string; name: string; installed: boolean; path: string | null; connected: boolean }
export interface ModelSource { providerId: string; providerName: string; mark: string; planName?: string | null; connectionName?: string | null; modelFamily?: string | null; available?: boolean }
export interface ModelCatalog { data: Model[]; source?: ModelSource }
export interface AgentProfile { id: string; agent: string; name: string; model: string | null; hasCredential: boolean; current: boolean; officialAccount?: boolean; accountId?: string | null; providerId?: string | null; protocol?: string; plan?: string | null; source?: string; models?: string[]; modelSource?: ModelSource }
export interface SidebarItem { pinned: boolean; unread: boolean; sectionId: string | null }
export interface SidebarSection { id: string; name: string }
export interface SidebarState { projects: Record<string, SidebarItem>; sessions: Record<string, SidebarItem>; sections: SidebarSection[] }
export interface ArchivedSession { session: Session; projectName: string; projectPath: string }
export interface Bootstrap { projects: Project[]; sessions: Session[]; agents: Agent[]; codexPath: string | null; loadMcp: boolean; profiles: AgentProfile[]; officialAgents: string[]; connectionOrder?: Record<string, string[]>; sidebar?: SidebarState }
export interface ProviderPreset { id: string; name: string; protocol: string; baseUrl: string; models: string[]; docsUrl?: string; note?: string }
export interface Provider { id: string; name: string; mark: string; category: string; presets: ProviderPreset[] }
export interface ProviderSettings { id: string | null; name: string; agent: string; providerId: string; plan: string; protocol: string; baseUrl: string; model: string; models: string[]; apiKey?: string | null; hasCredential: boolean; official?: boolean }
export type PermissionMode = 'read' | 'ask' | 'strict' | 'auto' | 'edit' | 'deny' | 'full';
export interface Model { id: string; model: string; displayName: string; isDefault: boolean; defaultReasoningEffort?: string; supportedReasoningEfforts?: { reasoningEffort: string; description?: string }[] }
export interface Changes { isGit: boolean; branch: string; files: { path: string; status: string }[]; diff: string }
export interface RpcEvent { id?: string | number; method: string; params: Record<string, any> }
export interface RuntimeInfo { running: boolean; pid: number | null; shellBytes: number; agentBytes: number; requests: RpcEvent[]; idleReleaseSeconds: number; memoryMetric: string }
export const isActive = (status?: string) => ['starting', 'running', 'waiting'].includes(status ?? '');
