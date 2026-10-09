import catalog from '../resources/providers.json';
import type { AgentProfile, Model, ModelSource } from './types';
import { nativeConnectionSource, visibleProviderIds, type ConnectionOrder } from './providerOrder';

export interface ModelOption extends Model {
  key: string;
  connectionId: string;
  source: ModelSource;
  rawIds: string[];
  context?: string;
  selected: boolean;
}
export interface ModelGroup { id: string; source: ModelSource; options: ModelOption[] }

export function sourceForProfile(profile: AgentProfile): ModelSource {
  if (profile.officialAccount && profile.accountId) return { ...nativeConnectionSource(profile.agent, '@official'), connectionName: profile.name };
  if (profile.agent === 'codex' && profile.officialAccount) return nativeConnectionSource('codex', '@official');
  if (profile.modelSource) return profile.modelSource;
  const provider = catalog.find(p => p.id === profile.providerId);
  const coding = provider?.id === 'kimi' && profile.plan === 'coding';
  return {
    providerId: provider?.id ?? 'custom',
    providerName: coding ? 'Kimi Code' : provider?.name ?? profile.name,
    mark: provider?.id === 'custom' || !provider ? Array.from(profile.name.trim())[0]?.toUpperCase() ?? '?' : provider.mark,
    planName: coding ? '编程计划' : provider?.presets.find(p => p.id === profile.plan)?.name,
    connectionName: profile.name,
  };
}

export function sourceLabel(source: ModelSource): string {
  const pieces = [source.providerName, source.planName];
  const connection = source.connectionName?.trim();
  const label = pieces.filter(Boolean).join(' · ');
  // Keep custom connection names visible without repeating “Kimi” twice.
  if (connection && ![label, source.providerId].some(p => p.toLowerCase().includes(connection.toLowerCase()))) pieces.push(connection);
  return pieces.filter(Boolean).join(' · ');
}

function modelIdentity(id: string, source: ModelSource) {
  // Claude's [1m] suffix configures its context window; it isn't another Kimi model.
  return isKimiSource(source) && /^k3(?:\[1m\])?$/i.test(id) ? 'k3' : id;
}
export function isKimiSource(source?: ModelSource) { return source?.providerId === 'kimi' || source?.modelFamily === 'kimi'; }

export function modelName(model: Model, source?: ModelSource) {
  if (isKimiSource(source)) {
    if (/^k3(?:\[1m\])?$/i.test(model.model)) return 'Kimi K3';
    if (model.model === 'k3-256k') return 'Kimi K3';
    if (model.model === 'kimi-for-coding') return 'Kimi for Coding';
    if (model.model === 'kimi-for-coding-highspeed') return 'Kimi for Coding 高速版';
  }
  return model.displayName || model.model;
}

function optionsFor(models: Model[], source: ModelSource, connectionId: string, current: boolean, value: string): ModelOption[] {
  const options = new Map<string, ModelOption>();
  for (const model of models) {
    if ('hidden' in model && model.hidden) continue;
    const identity = modelIdentity(model.model, source);
    const existing = options.get(identity);
    const selected = current && modelIdentity(value, source) === identity;
    const context = /\[1m\]$/i.test(model.model) && isKimiSource(source) ? '1M' : model.model === 'k3-256k' && isKimiSource(source) ? '256K' : undefined;
    if (!existing) options.set(identity, { ...model, key: `${connectionId}:${identity}`, connectionId, source, displayName: modelName(model, source), rawIds: [model.model], context, selected });
    else {
      if (!existing.rawIds.includes(model.model)) existing.rawIds.push(model.model);
      existing.context ??= context;
      // Preserve the exact selected CLI ID; otherwise prefer the configured default.
      if (model.model === value && current || model.isDefault && !existing.isDefault && !(current && existing.model === value)) {
        existing.model = model.model; existing.defaultReasoningEffort = model.defaultReasoningEffort;
        existing.supportedReasoningEfforts = model.supportedReasoningEfforts ?? existing.supportedReasoningEfforts;
      }
      existing.isDefault ||= model.isDefault;
    }
  }
  return [...options.values()].map(option => {
    if (option.rawIds.some(id => /^k3\[1m\]$/i.test(id)) && isKimiSource(source)) option.context = /\[1m\]$/i.test(option.model) ? '1M' : undefined;
    return option;
  });
}

export function modelGroups({ models, profiles, activeProfile, source, agent, value, connectionId, order, loggedInAgents = [] }: { models: Model[]; profiles: AgentProfile[]; activeProfile?: AgentProfile; source?: ModelSource; agent: string; value: string; connectionId?: string; order?: ConnectionOrder; loggedInAgents?: string[] }): ModelGroup[] {
  const currentId = connectionId ?? activeProfile?.id ?? '';
  const currentProfile = profiles.find(p => p.agent === agent && p.id === currentId);
  const legacyOfficial = agent === 'codex' && currentId === '@local' && source?.available === true && source.connectionName === 'ChatGPT 官方账号';
  const officialRoute = ['claude', 'codex'].includes(agent) && (currentId === '@official' || !!currentProfile?.officialAccount || legacyOfficial);
  const officialCurrent = agent === 'codex' && officialRoute && !currentProfile?.accountId;
  const accountKey = currentProfile?.accountId ? currentId : agent;
  const verifiedAgents = officialRoute && source?.available === false ? loggedInAgents.filter(id => id !== accountKey) : officialRoute && source?.available === true ? [...new Set([...loggedInAgents, accountKey])] : loggedInAgents;
  const currentAvailable = source?.available !== false && (officialRoute ? verifiedAgents.includes(accountKey) : !!currentProfile && !currentProfile.officialAccount);
  const currentSource = officialCurrent ? { ...nativeConnectionSource(agent, '@official'), available: source?.available } : source ?? (currentProfile ? sourceForProfile(currentProfile) : nativeConnectionSource(agent, currentId));
  const currentGroupId = officialCurrent ? '@official' : currentId;
  const profileModels = (profile: AgentProfile): Model[] => [...new Set([profile.model, ...profile.models ?? []].filter((m): m is string => !!m))].map(id => ({ id, model: id, displayName: id, isDefault: id === profile.model }));
  const officialModels = profiles.filter(p => p.agent === agent && p.officialAccount && !p.accountId).flatMap(profileModels);
  const currentModels = !currentAvailable ? [] : models.length ? models : currentProfile ? profileModels(currentProfile) : officialCurrent ? officialModels : [];
  const current = { id: currentGroupId, source: currentSource, options: optionsFor(currentModels, currentSource, currentId, true, value || currentProfile?.model || '') };
  const ids = visibleProviderIds(agent, profiles, verifiedAgents, order).filter(id => id !== currentGroupId || currentAvailable);
  if (currentAvailable && !ids.includes(currentGroupId)) ids.push(currentGroupId);
  if (!order?.[agent] && currentAvailable) { const index = ids.indexOf(currentGroupId); ids.splice(index, 1); ids.unshift(currentGroupId); }
  return ids.map(id => {
    if (id === currentGroupId) return current;
    const profile = profiles.find(p => p.agent === agent && p.id === id);
    const officialProfile = id === '@official' ? profiles.find(p => p.agent === agent && p.officialAccount && !p.accountId && (p.model || p.models?.length)) : undefined;
    const groupSource = profile ? sourceForProfile(profile) : nativeConnectionSource(agent, id);
    const groupModels = profile ? profileModels(profile) : id === '@official' && agent === 'codex' ? officialModels : officialProfile ? profileModels(officialProfile) : [];
    return { id, source: groupSource, options: optionsFor(groupModels, groupSource, id, false, '') };
  }).filter(g => g.id === currentGroupId || g.options.length > 0);
}

export function filterModelGroups(groups: ModelGroup[], search: string, connectionId: string | null): ModelGroup[] {
  const query = search.trim().toLowerCase();
  return groups.filter(g => connectionId === null || g.id === connectionId).map(g => ({ ...g, options: g.options.filter(m => `${m.displayName} ${m.rawIds.join(' ')} ${sourceLabel(m.source)} ${m.context ?? ''}`.toLowerCase().includes(query)) })).filter(g => g.options.length > 0);
}
