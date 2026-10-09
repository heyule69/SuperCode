export interface Attachment { kind: 'file' | 'image' | 'directory' | 'skill'; path: string; name: string; size?: number }
export const MAX_ATTACHMENTS = 12;
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
export const MAX_TOTAL_IMAGE_BYTES = 20 * 1024 * 1024;
const imageTypes = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
export function imagePath(path: string) { return /\.(png|jpe?g|gif|webp)$/i.test(path); }

export function clipboardImages(data: Pick<DataTransfer, 'items' | 'files'>): File[] {
  const items = Array.from(data.items ?? []).filter(item => item.kind === 'file').map(item => item.getAsFile()).filter((file): file is File => !!file);
  return (items.length ? items : Array.from(data.files ?? [])).filter(file => file.type.startsWith('image/') || imagePath(file.name));
}

export function validateImageFile(file: Pick<File, 'type' | 'name' | 'size'>) {
  if (!imageTypes.has(file.type) && !(file.type === '' && imagePath(file.name))) throw new Error('仅支持 PNG、JPEG、GIF、WebP 图片');
  if (!file.size) throw new Error('图片为空，请重新复制或选择图片');
  if (file.size > MAX_IMAGE_BYTES) throw new Error('图片不能超过 10 MB');
}

export function imageBytes(attachments: Attachment[]) {
  return attachments.reduce((total, item) => total + (item.kind === 'image' && Number.isFinite(item.size) && item.size! > 0 ? item.size! : 0), 0);
}

export function mergeAttachments(current: Attachment[], next: Attachment[]): Attachment[] {
  const merged = [...current];
  for (const item of next) if (!merged.some(existing => existing.path === item.path)) merged.push(item);
  if (merged.length > MAX_ATTACHMENTS) throw new Error('最多添加 12 个附件，请先移除一个附件');
  if (imageBytes(merged) > MAX_TOTAL_IMAGE_BYTES) throw new Error('图片总量不能超过 20 MB');
  return merged;
}

export function imageBase64(file: File): Promise<string> {
  validateImageFile(file);
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => { const result = String(reader.result ?? ''); const comma = result.indexOf(','); if (comma < 0) reject(new Error('无法读取图片')); else resolve(result.slice(comma + 1)); };
    reader.onerror = () => reject(new Error('无法读取图片，请重新选择'));
    reader.onabort = () => reject(new Error('图片读取已取消'));
    reader.readAsDataURL(file);
  });
}

export async function readClipboardImages(): Promise<File[]> {
  if (!navigator.clipboard?.read) return [];
  const items = await navigator.clipboard.read();
  const files: File[] = [];
  for (const item of items) {
    const type = item.types.find(type => imageTypes.has(type));
    if (type) files.push(new File([await item.getType(type)], `粘贴的图片.${type === 'image/jpeg' ? 'jpg' : type.slice(6)}`, { type }));
  }
  return files;
}
