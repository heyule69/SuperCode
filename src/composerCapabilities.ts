import type { ExtensionPlugin } from './extensions';
import type { Skill } from './skills';

const pathKey = (path: string) => path.replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\(?=[a-z]:\\)/i, '').replace(/\\/g, '/').replace(/\/+$/, '').toLocaleLowerCase();
export function pluginContainsPath(plugin: ExtensionPlugin, path: string) {
  const root = pathKey(plugin.path);
  return !!root && pathKey(path).startsWith(`${root}/`);
}
export function pluginContainsSkill(plugin: ExtensionPlugin, skill: Skill) {
  return [skill.path, ...(skill.paths ?? [])].some(path => pluginContainsPath(plugin, path));
}
export function skillMatches(skill: Skill, search: string) {
  return `${skill.name} ${skill.description ?? ''} ${skill.namespace ?? ''}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase());
}
export function composerCapabilities(skills: Skill[], plugins: ExtensionPlugin[], agent: string, search: string) {
  const installed = plugins.filter(plugin => plugin.installed);
  return {
    plugins: installed.filter(plugin => plugin.agent === agent || plugin.agent === 'both').map(plugin => {
      const matchesPlugin = `${plugin.name} ${plugin.description}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase());
      return { plugin, skills: skills.filter(skill => pluginContainsSkill(plugin, skill) && (matchesPlugin || skillMatches(skill, search))).map(skill => {
        const path = [skill.path, ...(skill.paths ?? [])].find(path => pluginContainsPath(plugin, path))!;
        return path === skill.path ? skill : { ...skill, path };
      }) };
    }).filter(({ plugin, skills }) => !search.trim() || `${plugin.name} ${plugin.description}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()) || skills.length > 0),
    skills: skills.filter(skill => !skill.namespace && [skill.path, ...(skill.paths ?? [])].some(path => !installed.some(plugin => pluginContainsPath(plugin, path))) && skillMatches(skill, search)).map(skill => {
      const path = [skill.path, ...(skill.paths ?? [])].find(path => !installed.some(plugin => pluginContainsPath(plugin, path)))!;
      return path === skill.path ? skill : { ...skill, path };
    }),
  };
}
