import { call } from './api';

export interface Skill { name: string; description?: string; path: string; source?: string; sources?: string[]; namespace?: string | null; error?: string | null }
export interface SkillCatalog { skills: Skill[]; warnings?: string[] }
const cache = new Map<string, { at: number; catalog: SkillCatalog }>();
const pending = new Map<string, Promise<SkillCatalog>>();
export function loadLocalSkills(projectId: string, force = false): Promise<SkillCatalog> {
  const saved = cache.get(projectId);
  if (!force && saved && Date.now() - saved.at < 30_000) return Promise.resolve(saved.catalog);
  const running = pending.get(projectId); if (running) return running;
  const request = call<SkillCatalog>('list_local_skills', { projectId: projectId || null }).then(catalog => {
    cache.delete(projectId); cache.set(projectId, { at: Date.now(), catalog });
    if (cache.size > 4) cache.delete(cache.keys().next().value!);
    window.dispatchEvent(new CustomEvent('supercode:skills-updated', { detail: { projectId, catalog } }));
    return catalog;
  }).finally(() => pending.delete(projectId));
  pending.set(projectId, request); return request;
}
export function skillSource(skill: Skill) { return skill.sources?.join(' · ') || skill.source || '本机技能'; }
