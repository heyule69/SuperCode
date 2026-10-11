import { readFile, readdir, mkdir, unlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { basename, dirname, resolve } from 'node:path';
import { generateManifest } from 'material-icon-theme';
import type { Plugin } from 'vite';

// Only the upstream associations and SVG assets enter the app; its generator runs at build time.
export function fileIconTheme(): Plugin {
  const id = 'virtual:material-file-icons';
  const manifest = generateManifest({ activeIconPack: 'react' });
  const fields = ['file', 'folder', 'folderExpanded', 'fileNames', 'fileExtensions', 'folderNames', 'folderNamesExpanded'] as const;
  const normalize = (value: string | Record<string, string> | undefined) => typeof value === 'object' ? Object.fromEntries(Object.entries(value).map(([key, icon]) => [key.toLowerCase(), icon])) : value;
  const associations = Object.fromEntries(fields.map(key => [key, normalize(manifest[key])]));
  const lightAssociations = Object.fromEntries(fields.map(key => [key, normalize(manifest.light?.[key])]));
  const used = new Set<string>();
  for (const theme of [associations, lightAssociations]) {
    for (const value of Object.values(theme)) {
      if (typeof value === 'string') used.add(value);
      else if (value) Object.values(value).forEach(icon => used.add(icon));
    }
  }
  const definitions = Object.fromEntries(Object.entries(manifest.iconDefinitions ?? {}).filter(([name]) => used.has(name)).map(([name, icon]) => [name, basename(icon.iconPath)]));
  const catalog = { ...associations, light: lightAssociations, definitions };
  const require = createRequire(import.meta.url);
  const packageRoot = dirname(require.resolve('material-icon-theme/package.json'));
  function compact(value: unknown): string {
    if (!value || typeof value !== 'object') return JSON.stringify(value) ?? 'undefined';
    if ('light' in value || 'definitions' in value) return `{${Object.entries(value).map(([key, item]) => `${JSON.stringify(key)}:${key === 'definitions' ? JSON.stringify(item) : compact(item)}`).join(',')}}`;
    if (Object.values(value).every(item => typeof item === 'string')) {
      const groups = new Map<string, string[]>();
      for (const [name, icon] of Object.entries(value)) { const names = groups.get(icon) ?? []; names.push(name); groups.set(icon, names); }
      return `expand(${JSON.stringify([...groups])})`;
    }
    return `{${Object.entries(value).map(([key, item]) => `${JSON.stringify(key)}:${compact(item)}`).join(',')}}`;
  }
  return {
    name: 'supercode-file-icons',
    async configResolved(config) {
      if (!config.publicDir) throw new Error('File icons require a public assets directory');
      const target = resolve(config.publicDir, 'file-icons');
      await mkdir(target, { recursive: true });
      const decoder = new TextDecoder('utf-8', { fatal: true });
      // Bounded concurrency avoids opening a thousand files at once on Windows.
      const assets = [...new Set(Object.values(definitions))];
      const assetNames = new Set(assets);
      for (const entry of await readdir(target, { withFileTypes: true, encoding: 'utf8' })) {
        if (entry.isFile() && entry.name.endsWith('.svg') && !assetNames.has(entry.name)) await unlink(resolve(target, entry.name));
      }
      for (let offset = 0; offset < assets.length; offset += 32) {
        await Promise.all(assets.slice(offset, offset + 32).map(async file => {
          const bytes = await readFile(resolve(packageRoot, 'icons', file));
          if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) throw new Error(`Upstream icon has UTF-8 BOM: ${file}`);
          await writeFile(resolve(target, file), decoder.decode(bytes), 'utf8');
        }));
      }
      await writeFile(resolve(target, 'LICENSE.txt'), await readFile(resolve(packageRoot, 'LICENSE'), 'utf8'), 'utf8');
    },
    resolveId(source) { if (source === id) return '\0' + id; },
    load(source) {
      if (source === '\0' + id) return `const expand = groups => Object.fromEntries(groups.flatMap(([icon, names]) => names.map(name => [name, icon]))); export default ${compact(catalog)};`;
    },
  };
}
