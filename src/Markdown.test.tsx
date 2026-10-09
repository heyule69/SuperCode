import { expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import Markdown from './Markdown';

it('makes displayed file paths actionable without changing the displayed text', () => {
  const html = renderToStaticMarkup(<Markdown text={'文件位置：`D:\\HYLcode\\My Folder\\报告.pdf`'} openFile={() => {}}/>);
  expect(html).toContain('class="markdown-file-link"');
  expect(html).toContain('data-resource-path="D:\\HYLcode\\My Folder\\报告.pdf"');
  expect(html).toContain('<code>D:\\HYLcode\\My Folder\\报告.pdf</code>');
});
it('adds resource icons to titled links while leaving ordinary inline code alone', () => {
  const html = renderToStaticMarkup(<Markdown text={'[验证记录](docs/check.md) · [桌面截图](preview.png) · [网站](https://example.com)\n\n`const a = 1`'}/>);
  expect(html.match(/class="markdown-link /g)).toHaveLength(3);
  expect(html).toContain('lucide-file-text');
  expect(html).toContain('lucide-image');
  expect(html).toContain('lucide-globe');
  expect(html).toContain('<code>const a = 1</code>');
});
