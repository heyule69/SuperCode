import type { AgentProfile } from './types';
import type { Draft, DraftModel } from './workspaceState';

const prefix = 'supercode.model-selection.v1:';
const hasConnection = (agent: string, selection: DraftModel, profiles: AgentProfile[], officialAgents: string[]) =>
  profiles.some(profile => profile.agent === agent && profile.id === selection.connectionId)
  || selection.connectionId === '@official' && officialAgents.includes(agent);

export function loadModelSelection(agent: string, profiles: AgentProfile[], officialAgents: string[]): DraftModel | undefined {
  try {
    const value = JSON.parse(localStorage.getItem(prefix + agent) ?? 'null');
    if (!value || typeof value.connectionId !== 'string' || typeof value.model !== 'string' || !value.model
      || value.connectionId.length > 256 || value.model.length > 256 || typeof value.effort !== 'string' || value.effort.length > 64) return;
    if (!hasConnection(agent, value, profiles, officialAgents)) return;
    return { connectionId: value.connectionId, model: value.model, effort: value.effort };
  } catch { return; }
}

export function saveModelSelection(agent: string, selection: DraftModel) {
  localStorage.setItem(prefix + agent, JSON.stringify({ connectionId: selection.connectionId, model: selection.model, effort: selection.effort ?? '' }));
}

export function initialModelSelection(agent: string, profiles: AgentProfile[], officialAgents: string[], draft?: Draft, fresh = false) {
  const recent = loadModelSelection(agent, profiles, officialAgents);
  const selection = draft?.selection && hasConnection(agent, draft.selection, profiles, officialAgents) ? draft.selection : undefined;
  // An unfinished draft retains its own options; a fresh window prefers the last explicit choice.
  return !fresh && (draft?.text || draft?.attachments.length) ? selection ?? recent : recent ?? selection;
}
