import catalog from 'virtual:material-file-icons';

export interface IconAssociations {
  file?: string; folder?: string; folderExpanded?: string;
  fileNames?: Record<string, string>; fileExtensions?: Record<string, string>;
  folderNames?: Record<string, string>; folderNamesExpanded?: Record<string, string>;
}
export interface FileIconCatalog extends IconAssociations { light: IconAssociations; definitions: Record<string, string> }
function own(map: Record<string, string> | undefined, key: string) {
  return map && Object.hasOwn(map, key) ? map[key] : undefined;
}
function named(map: Record<string, string> | undefined, path: string, fallback?: Record<string, string>) {
  let suffix = path;
  while (suffix) {
    const match = own(map, suffix) ?? own(fallback, suffix);
    if (match) return match;
    const slash = suffix.indexOf('/');
    if (slash < 0) return;
    suffix = suffix.slice(slash + 1);
  }
}
function extension(map: Record<string, string> | undefined, name: string, fallback?: Record<string, string>, parent = '') {
  for (const prefix of parent ? [parent + '/', ''] : ['']) {
    for (let dot = name.indexOf('.'); dot >= 0; dot = name.indexOf('.', dot + 1)) {
      const key = prefix + name.slice(dot + 1);
      const match = own(map, key) ?? own(fallback, key);
      if (match) return match;
    }
  }
}
export function fileIcon(name: string, directory = false, expanded = false, light = false) {
  const path = name.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  const slash = path.lastIndexOf('/');
  const base = path.slice(slash + 1);
  const parent = slash < 0 ? '' : path.slice(0, slash).split('/').pop() ?? '';
  const theme = light ? catalog.light : undefined;
  const names = directory ? expanded ? 'folderNamesExpanded' : 'folderNames' : 'fileNames';
  let icon = named(theme?.[names], path, catalog[names]);
  if (!icon && !directory) icon = extension(theme?.fileExtensions, base, catalog.fileExtensions, parent);
  if (!icon) {
    const fallback = directory ? expanded ? 'folderExpanded' : 'folder' : 'file';
    icon = theme?.[fallback] ?? catalog[fallback] ?? (directory ? 'folder' : 'file');
  }
  return { id: icon, url: `${import.meta.env.BASE_URL}file-icons/${catalog.definitions[icon]}` };
}
