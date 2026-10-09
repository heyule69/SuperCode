import { useEffect, useState } from 'react';
import { Box, File, Folder, Image, LoaderCircle, Plus, Sparkles, X } from 'lucide-react';
import { call, desktop } from './api';
import type { Skill } from './ClientSettings';
import { useFloatingLayer } from './useFloatingLayer';
import { imagePath, type Attachment } from './attachments';
import { AttachmentImage } from './AttachmentImage';
export type { Attachment } from './attachments';
export function AddMenu({ addAttachments, project, busy, openSkills }: { addAttachments: (items: Attachment[]) => Promise<void>; project: () => void; busy: boolean; openSkills: () => void }) {
  const [open, setOpen] = useState(false); const [error, setError] = useState(''); const [picking, setPicking] = useState(false);
  const layer = useFloatingLayer(open, () => setOpen(false), { focusFirst: true });
  useEffect(() => { if (busy) setOpen(false); }, [busy]);
  async function pick(kind: 'file' | 'image' | 'directory') {
    layer.dismiss(true); setError(''); if (!desktop) { setError('桌面版可添加本机文件'); return; }
    setPicking(true);
    try { const { open: dialog } = await import('@tauri-apps/plugin-dialog'); const selected = await dialog({ multiple: kind !== 'directory', directory: kind === 'directory', title: kind === 'image' ? '添加图片' : kind === 'directory' ? '添加目录' : '添加文件', filters: kind === 'image' ? [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }] : undefined }); if (!selected) return; const paths = Array.isArray(selected) ? selected : [selected];
      const next = paths.map(path => ({ kind: kind === 'file' && imagePath(path) ? 'image' as const : kind, path, name: path.split(/[\\/]/).filter(Boolean).pop() ?? path }));
      await addAttachments(next);
    } catch (e) { setError(String(e)); } finally { setPicking(false); }
  }
  return <div className="composer-menu-anchor" ref={layer.root} onKeyDown={layer.navigate}><button type="button" className="icon-button" title="添加文件、图片、目录或技能" aria-label="添加附件" aria-haspopup="menu" aria-expanded={open} disabled={busy || picking} onClick={() => setOpen(v => !v)} onKeyDown={e => { if (e.key === 'ArrowDown') { e.preventDefault(); setOpen(true); } }}><Plus size={18} /></button>{open ? <div className="composer-popover add-popover" role="menu"><button type="button" role="menuitem" onClick={() => void pick('file')}><File size={15} />文件</button><button type="button" role="menuitem" onClick={() => void pick('image')}><Image size={15} />图片</button><button type="button" role="menuitem" onClick={() => void pick('directory')}><Folder size={15} />目录</button><button type="button" role="menuitem" onClick={() => { setOpen(false); openSkills(); }}><Sparkles size={15} />技能</button><hr /><button type="button" role="menuitem" onClick={() => { setOpen(false); project(); }}><Plus size={15} />添加项目</button></div> : null}{error ? <div className="attachment-error" role="alert"><span>{error}</span><button type="button" className="icon-button" aria-label="关闭附件提示" onClick={() => setError('')}><X size={13} /></button></div> : null}</div>;
}
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
