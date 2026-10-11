import { Box, File, Folder, LoaderCircle, X } from 'lucide-react';
import { call } from './api';
import type { Skill } from './ClientSettings';
import { type Attachment } from './attachments';
import { AttachmentImage } from './AttachmentImage';
export type { Attachment } from './attachments';
export { AddMenu } from './ComposerAddMenu';
export function ComposerSkills({ attachments, remove }: { attachments: Attachment[]; remove: (path: string) => void }) {
  const skills = attachments.filter(a => a.kind === 'skill');
  return skills.length ? <div className="composer-skills" role="list" aria-label="已选择技能">{skills.map(skill => <div className="composer-skill" role="listitem" key={skill.path} title={`${skill.name} · 在输入文字开头按退格键移除技能`}>
      <Box size={17} aria-hidden="true" /><span>{skill.name.charAt(0).toUpperCase() + skill.name.slice(1)}</span>
      <button type="button" className="remove-skill" aria-label={`移除技能 ${skill.name}`} title="移除技能" onClick={() => remove(skill.path)}><X size={13} /></button>
    </div>)}</div> : null;
}
export function AttachmentStrip({ attachments, remove, loading = false }: { attachments: Attachment[]; remove: (path: string) => void; loading?: boolean }) {
  const files = attachments.filter(a => a.kind !== 'skill');
  return files.length || loading ? <div className="attachment-strip">{files.map(a => a.kind === 'image' ? <div className="image-attachment-card" key={a.path}><AttachmentImage attachment={a}/><button type="button" className="remove-image" aria-label={`移除图片 ${a.name}`} title="移除图片" onClick={() => remove(a.path)}><X size={12}/></button></div> : <div key={a.path} title={a.path}>{a.kind === 'directory' ? <Folder size={13} /> : <File size={13} />}<span>{a.name}</span><button type="button" aria-label={`移除附件 ${a.name}`} onClick={() => remove(a.path)}><X size={12} /></button></div>)}{loading ? <div className="attachment-importing" role="status"><LoaderCircle size={14} className="spin"/>正在添加图片…</div> : null}</div> : null;
}
export { PermissionMenu } from './PermissionMenu';
export { permissionNames } from './permissions';
export { ModelMenu } from './ModelMenu';
export async function chooseSkills(projectId: string): Promise<Skill[]> { return (await call<{ skills: Skill[] }>('list_local_skills', { projectId: projectId || null })).skills; }
