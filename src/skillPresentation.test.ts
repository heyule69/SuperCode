import { expect, it } from 'vitest';
import { selectedSkills, skillLabel, skillMessageText } from './skillPresentation';
const skill = {kind:'skill',name:'save',path:'D:/skills/save/SKILL.md'};
it('presents selected skills while preserving the transport text and ordinary user instructions',()=>{
  const attachments=[skill,{kind:'file',name:'预算表.xlsx',path:'D:/预算表.xlsx'}];
  expect(skillMessageText('请执行所选技能。',attachments)).toBe('');
  expect(skillMessageText('请执行所选技能 save。',attachments)).toBe('');
  expect(skillMessageText('请保存这个项目的状态',attachments)).toBe('请保存这个项目的状态');
  expect(skillMessageText('请执行所选技能。',[])).toBe('请执行所选技能。');
  expect(attachments[0]).toEqual(skill);
});
it('recovers historical skill labels from paths without reading any files',()=>{
  const old={kind:'skill',name:'SKILL.md',path:'C:\\skills\\save\\SKILL.md'};
  expect(skillLabel(old)).toBe('Save');
  expect(skillMessageText('请执行所选技能 save。',[old])).toBe('');
  expect(skillLabel({kind:'skill',name:'技能名称很长'})).toBe('技能名称很长');
  expect(selectedSkills([skill,skill])).toHaveLength(1);
});
