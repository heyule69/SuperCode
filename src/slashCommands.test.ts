import { describe, expect, it } from 'vitest';
import { commandMatches, parseCommand } from './slashCommands';
describe('agent command routing', () => {
  it('offers goals for Codex and discovers namespaced Claude commands', () => {
    expect(commandMatches('/', 'claude').some(c => c.name === '/goal')).toBe(false);
    expect(commandMatches('/go', 'codex').map(c => c.name)).toEqual(['/goal', '/goal-status']);
    expect(commandMatches('/goal create something', 'codex')).toEqual([]);
    expect(commandMatches('/compact', 'claude').map(c => c.name)).toEqual(['/compact']);
    expect(commandMatches('/plugin:', 'claude', ['plugin:review', 'compact', '../../bad']).map(c => c.name)).toEqual(['/plugin:review']);
    expect(parseCommand('/plugin:review src')).toEqual({ command: '/plugin:review', argument: 'src' });
  });
  it('keeps multiline goal objectives and avoids matching shell or prose text', () => {
    expect(parseCommand('/goal Build an app\nwith tests')).toEqual({ command: '/goal', argument: 'Build an app\nwith tests' });
    expect(parseCommand('please run /goal')).toBeNull();
    expect(parseCommand('/goal; rm')).toBeNull();
  });
  it('offers local skills in both Agents and gives skills priority over native aliases', () => {
    const skills = [{name:'recall',path:'C:/skills/recall/SKILL.md',description:'恢复项目状态'}, {name:'save',path:'C:/skills/save/SKILL.md',description:'保存项目状态'}];
    for (const agent of ['claude','codex']) {
      expect(commandMatches('/re',agent,['recall'],skills).map(c=>c.name)).toEqual(['/recall']);
      expect(commandMatches('/save',agent,[],skills)[0].skill?.path).toBe(skills[1].path);
      expect(commandMatches('/recall task',agent,[],skills)).toEqual([]);
    }
  });
  it('keeps distinct skill paths selectable and never shadows built-in commands', () => {
    const skills=[{name:'status',path:'one'},{name:'save',path:'two'},{name:'save',path:'three'},{name:'save',path:'four',namespace:'plugin'},{name:'bad',path:'five',error:'not UTF-8'}];
    const commands=commandMatches('/','codex',[],skills);
    expect(commands.find(c=>c.name==='/status')?.skill).toBeUndefined();
    expect(commands.filter(c=>c.skill).map(c=>c.name)).toEqual(['/skill:status','/save','/skill:save','/plugin:save']);
    expect(parseCommand('/plugin:save 请保存\n项目状态')).toEqual({command:'/plugin:save',argument:'请保存\n项目状态'});
  });
});
