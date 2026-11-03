import { Box } from 'lucide-react';
import { selectedSkills, skillLabel, type SkillReference } from './skillPresentation';
import './skill-badges.css';

export function SkillBadges({ attachments }: { attachments: readonly SkillReference[] }) {
  const skills = selectedSkills(attachments);
  return skills.length ? <span className="skill-badges" role="list" aria-label="消息技能">{skills.map((skill, index) =>
    <span className="skill-badge" role="listitem" key={skill.path || `${skill.name}-${index}`} title={skill.path ? `${skillLabel(skill)} · ${skill.path}` : skillLabel(skill)}>
      <Box size={16} aria-hidden="true"/><span>{skillLabel(skill)}</span>
    </span>
  )}</span> : null;
}
