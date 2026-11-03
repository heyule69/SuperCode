export interface SkillReference { kind: string; name?: string; path?: string }

export function selectedSkills(attachments: readonly SkillReference[]) {
  return [...new Map(attachments.filter(a => a.kind === 'skill').map(a => [a.path || a.name, a])).values()];
}

export function skillLabel(skill: SkillReference) {
  const parent = skill.path?.replace(/\\/g, '/').split('/').filter(Boolean).at(-2);
  const name = !skill.name || /^skill\.md$/i.test(skill.name) ? parent || '技能' : skill.name;
  return name.charAt(0).toUpperCase() + name.slice(1);
}

// These are transport defaults for attachment-only requests. Retain the actual
// payload/history, and present its selected skills instead of the boilerplate.
export function skillMessageText(text: string, attachments: readonly SkillReference[]) {
  const skills = selectedSkills(attachments);
  if (!skills.length) return text;
  const trimmed = text.trim();
  if (trimmed === '请执行所选技能。' || skills.some(skill => [skill.name, skillLabel(skill), skill.path?.replace(/\\/g,'/').split('/').filter(Boolean).at(-2)]
    .some(name => name && trimmed.toLowerCase() === `请执行所选技能 ${name}。`.toLowerCase()))) return '';
  return text;
}
