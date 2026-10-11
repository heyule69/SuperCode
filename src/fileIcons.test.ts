import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, it } from 'vitest';
import catalog from 'virtual:material-file-icons';
import { fileIcon } from './fileIcons';

it('uses the upstream filename and compound-extension associations before generic file types', () => {
  expect(fileIcon('C:\\项目\\PACKAGE.JSON').id).toBe('nodejs');
  expect(fileIcon('package-lock.json').id).toBe('nodejs');
  expect(fileIcon('tsconfig.json').id).toBe('tsconfig');
  expect(fileIcon('vite.config.ts').id).toBe('vite');
  expect(fileIcon('README.md').id).toBe('readme');
  expect(fileIcon('AGENTS.md').id).toBe('agent');
  expect(fileIcon('.gitignore').id).toBe('git');
  expect(fileIcon('claude.md').id).toBe('claude');
  expect(fileIcon('CMakePresets.json').id).toBe('cmake');
  expect(fileIcon('D:\\项目\\.github\\FUNDING.yml').id).toBe('github-sponsors');
  expect(fileIcon('src/types.d.ts').id).toBe('typescript-def');
  expect(fileIcon('src/main.ts').id).toBe('typescript');
  expect(fileIcon('src/App.tsx').id).toBe('react_ts');
  expect(fileIcon('src-tauri/src/lib.rs').id).toBe('rust');
  expect(new Set(['报告.docx', '数据.xlsx', '演示.pptx', '图片.png', 'main.py', 'index.html'].map(name => fileIcon(name).id)).size).toBe(6);
  expect(fileIcon('unknown.custom-format').id).toBe('file');
  expect(fileIcon('constructor').id).toBe('file');
});

it('distinguishes named folders, opened folders and theme variants', () => {
  for (const [name, icon] of [['src', 'src'], ['docs', 'docs'], ['node_modules', 'node'], ['public', 'public'], ['scripts', 'scripts'], ['.github', 'github'], ['src-tauri', 'src-tauri']]) {
    expect(fileIcon(name, true).id).toBe(`folder-${icon}`);
    expect(fileIcon(`D:\\项目\\${name}\\`, true, true).id).toBe(`folder-${icon}-open`);
  }
  expect(fileIcon('自定义目录', true).id).toBe('folder');
  expect(fileIcon('自定义目录', true, true).id).toBe('folder-open');
  expect(fileIcon('next.config.ts', false, false, true).id).toBe('next_light');
  expect(fileIcon('next.config.ts').id).toBe('next');
});

it('ships every catalog SVG and its license as local assets without external URLs or a BOM', () => {
  for (const name of new Set(Object.values(catalog.definitions))) {
    const path = resolve('public/file-icons', name);
    expect(existsSync(path), name).toBe(true);
    const svg = readFileSync(path, 'utf8');
    expect(svg.startsWith('\uFEFF'), name).toBe(false);
    expect(svg, name).toContain('<svg');
  }
  expect(fileIcon('App.tsx').url).toMatch(/^\/file-icons\/.*\.svg$/);
  expect(readFileSync('public/file-icons/LICENSE.txt', 'utf8')).toContain('Copyright (c) 2025 Material Extensions');
});
