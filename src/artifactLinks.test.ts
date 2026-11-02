import {expect,it} from 'vitest';
import {artifactLink,referencedArtifactLinks,turnArtifactLinks} from './artifactLinks';
import type {Message} from './types';

it('recognizes deployed websites, standalone URLs and local preview addresses without checking the network',()=>{
  const text='已部署：[项目主页](https://demo.example.com/)\n\n本地预览：`http://localhost:3000/demo`\n\nhttps://other.example.com\n\n[IPv6 预览](http://[::1]:8080/)';
  const links=referencedArtifactLinks(text);
  expect(links.map(link=>link.url)).toEqual(['https://demo.example.com/','http://localhost:3000/demo','https://other.example.com/','http://[::1]:8080/']);
  expect(links.filter(link=>link.local)).toHaveLength(2);
  expect(links.find(link=>link.title==='项目主页')?.kind).toBe('website');
});
it('distinguishes download links and keeps signed URL parameters out of the visible address',()=>{
  const link=referencedArtifactLinks('下载：[Windows 安装包](https://cdn.example.com/build.exe?token=private#section)')[0];
  expect(link).toMatchObject({kind:'download',title:'Windows 安装包',display:'cdn.example.com/build.exe'});
  expect(link.url).toContain('token=private');
  expect(referencedArtifactLinks('[下载报告](https://example.com/get?id=123)')[0]?.kind).toBe('download');
});
it('supports local service shorthand, a repository delivery and explicit page-open labels',()=>{
  const links=referencedArtifactLinks('本地预览：`localhost:3000/demo`\n\n[打开页面](https://demo.example.com/)\n\n源码仓库：[GitHub](https://github.com/example/project)');
  expect(links.map(link=>link.url)).toEqual(['http://localhost:3000/demo','https://demo.example.com/','https://github.com/example/project']);
});
it('handles autolinks and reference-style delivery links without promoting unused definitions',()=>{
  const links=referencedArtifactLinks('<https://demo.example.com/>\n\n网址：[在线预览][site]\n\n[site]: https://other.example.com/?a=1&amp;b=2\n[unused]: https://unused.example.com/');
  expect(links.map(link=>link.url)).toEqual(['https://demo.example.com/','https://other.example.com/?a=1&b=2']);
});
it.each([
  '参考：[官方文档](https://docs.example.com/)',
  '资料来源：\n\nhttps://docs.example.com',
  '这是一个[普通引用](https://example.com/reference)，不是交付地址。',
  '将来可以使用 [项目主页](https://example.com/)。',
  '尚未部署：[在线预览](https://example.com/)',
  '示例地址：[网站预览](https://example.com/)',
  '项目已生成。\n\n尚未部署：[在线预览](https://example.com/)',
  '[普通链接](https://example.com/) 不是交付地址。',
  '```md\n[预览](https://example.com/)\n```',
  '`[预览](https://example.com/)`',
  '<!--\n[预览](https://example.com/)\n-->',
  '![预览](https://example.com/preview)',
  '[截图预览](https://example.com/result.png)\n[视频下载](https://example.com/result.mp4)',
  '[1](https://example.com/)\n[2](https://other.example.com/)',
])('does not promote references, placeholders or already embedded media: %s',text=>{
  expect(referencedArtifactLinks(text)).toEqual([]);
});
it('handles encoded paths, parentheses, Chinese punctuation and duplicate canonical URLs',()=>{
  const links=referencedArtifactLinks('网站预览：[结果](<https://demo.example.com/中文目录/a(b)>)\n\n访问地址：https://demo.example.com/中文目录/a(b)。\n\nhttps://demo.example.com');
  expect(links).toHaveLength(2);
  expect(links[0].url).toBe('https://demo.example.com/%E4%B8%AD%E6%96%87%E7%9B%AE%E5%BD%95/a(b)');
  expect(referencedArtifactLinks('[在线预览](https://example.com/a(b))')[0]?.url).toBe('https://example.com/a(b)');
});
it.each(['javascript:alert(1)','data:text/html,x','file:///C:/a.html','mailto:x@example.com','https://user:secret@example.com','https://example.com/\nrun','not a URL'])('rejects unsafe or invalid web destinations: %s',source=>{
  expect(artifactLink(source)).toBeNull();
});
it('deduplicates the final response and bounds large collections',()=>{
  const text=Array.from({length:80},(_,i)=>`[预览 ${i}](https://example.com/${i})`).join('\n');
  const final={id:'final',seq:1,sessionId:'s',role:'assistant',kind:'agentMessage',text,data:{}} as Message;
  expect(turnArtifactLinks([final,final])).toHaveLength(64);
});
