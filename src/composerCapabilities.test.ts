import { expect, it } from 'vitest';
import { composerCapabilities, pluginContainsSkill } from './composerCapabilities';
import type { ExtensionPlugin } from './extensions';
import type { Skill } from './skills';

const plugin = (id: string, path: string, agent = 'codex', installed = true): ExtensionPlugin => ({ id, path, agent, installed, name: id, description: '文档工具', nativeId: id, enabled: true, version: '1', source: '', components: ['技能'], icon: null, limitation: null });
const skill = (name: string, path: string, namespace?: string): Skill => ({ name, path, namespace, description: '编写文档' });
it('matches Windows plugin paths at directory boundaries without including a similarly named plugin', () => {
  const docs = plugin('Documents', 'C:\\Plugins\\Documents\\');
  expect(pluginContainsSkill(docs, skill('write', 'c:/plugins/documents/skills/write/SKILL.md'))).toBe(true);
  expect(pluginContainsSkill(docs, skill('write', '\\\\?\\C:\\Plugins\\Documents\\skills\\write\\SKILL.md'))).toBe(true);
  expect(pluginContainsSkill(plugin('UNC', '\\\\server\\share\\docs'), skill('write', '\\\\?\\UNC\\server\\share\\docs\\skills\\write\\SKILL.md'))).toBe(true);
  expect(pluginContainsSkill(docs, skill('other', 'C:/Plugins/Documents-extra/skills/SKILL.md'))).toBe(false);
  expect(pluginContainsSkill(plugin('empty', ''), skill('x', '/skills/x'))).toBe(false);
});
it('retains both engine plugin copies after content deduplication and selects the current engine path', () => {
  const plugins = [plugin('Documents', 'C:/codex/docs'), plugin('Documents', 'C:/claude/docs', 'claude')];
  const deduped = { ...skill('documents', 'C:/codex/docs/skills/doc/SKILL.md', 'Documents'), paths: ['C:/codex/docs/skills/doc/SKILL.md', 'C:/claude/docs/skills/doc/SKILL.md'] };
  const claude = composerCapabilities([deduped], plugins, 'claude', '');
  expect(claude.plugins[0].skills).toHaveLength(1);
  expect(claude.plugins[0].skills[0].path).toBe('C:/claude/docs/skills/doc/SKILL.md');
  expect(claude.skills).toHaveLength(0);
  const standalone = { ...deduped, namespace: undefined, path: 'C:/local/doc/SKILL.md', paths: [...deduped.paths, 'C:/local/doc/SKILL.md'] };
  expect(composerCapabilities([standalone], plugins, 'claude', '').skills[0].path).toBe('C:/local/doc/SKILL.md');
});
it('keeps engine-specific plugins and their skills out of the standalone list', () => {
  const plugins = [plugin('Codex', 'C:/codex'), plugin('Claude', 'C:/claude', 'claude'), plugin('Uninstalled', 'C:/uninstalled', 'codex', false)];
  const skills = [skill('codex-doc', 'C:/codex/skills/x'), skill('claude-doc', 'C:/claude/skills/y'), skill('local', 'C:/skills/local'), skill('unknown-plugin', 'C:/missing/x', 'missing')];
  const codex = composerCapabilities(skills, plugins, 'codex', '');
  expect(codex.plugins.map(item => item.plugin.name)).toEqual(['Codex']);
  expect(codex.plugins[0].skills.map(item => item.name)).toEqual(['codex-doc']);
  expect(codex.skills.map(item => item.name)).toEqual(['local']);
});
it('finds plugins by nested skill metadata, and includes all skills when the plugin itself matches', () => {
  const docs = plugin('Documents', 'C:/docs');
  const skills = [skill('write', 'C:/docs/skills/write'), skill('review', 'C:/docs/skills/review')];
  expect(composerCapabilities(skills, [docs], 'codex', ' review ').plugins[0].skills.map(s => s.name)).toEqual(['review']);
  expect(composerCapabilities(skills, [docs], 'codex', 'DOCUMEN').plugins[0].skills).toEqual(skills);
  expect(composerCapabilities(skills, [docs], 'codex', 'missing').plugins).toEqual([]);
});
