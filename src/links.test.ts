import { expect, it } from 'vitest';
import { describeLink, resourceFromText, systemDocument } from './links';
it('opens only supported external link schemes and rejects embedded credentials', () => {
  expect(describeLink('https://example.com/docs').kind).toBe('external');
  for (const href of ['javascript:alert(1)', 'data:text/html,x', 'cmd:run', 'https://user:secret@example.com']) expect(describeLink(href).kind).toBe('unsupported');
});
it('recognizes standalone displayed paths, links and line references', () => {
  expect(resourceFromText(' D:\\HYLcode\\My Folder\\AI热点日报.pdf ')).toEqual({ kind: 'file', target: 'D:\\HYLcode\\My Folder\\AI热点日报.pdf', line: undefined });
  expect(resourceFromText('src/App.tsx:12')).toEqual({ kind: 'file', target: 'src/App.tsx', line: 12 });
  expect(resourceFromText('make_ai_news_pdf.py')?.kind).toBe('file');
  expect(resourceFromText('https://example.com/docs')?.kind).toBe('external');
});
it('does not turn ordinary code, credentials or executable URLs into resources', () => {
  for (const value of ['const a = 1', 'k3[1M]', 'application/json', 'npm run scripts/build.ts', 'cd D:/repo && node build.js', 'javascript:alert(1)', 'https://user:secret@example.com', 'line one\nline two']) expect(resourceFromText(value)).toBeNull();
  expect(systemDocument('AI热点日报.PDF')).toBe(true);
  expect(systemDocument('diagram.png')).toBe(true);
  expect(systemDocument('script.py')).toBe(false);
  expect(systemDocument('app.exe')).toBe(false);
});
it('keeps Chinese and spaced project paths and code line references', () => {
  expect(describeLink('D:/项目/My%20File.ts:12')).toEqual({ kind: 'file', target: 'D:/项目/My File.ts', line: 12 });
  expect(describeLink('file:///D:/repo/code.ts#L24')).toEqual({ kind: 'file', target: 'D:/repo/code.ts', line: 24 });
  expect(describeLink('src/App.tsx')).toEqual({ kind: 'file', target: 'src/App.tsx', line: undefined });
  expect(describeLink('#heading').kind).toBe('anchor');
});
