// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it } from 'vitest';
import { highlightCode, highlightedLines } from './codeHighlight';
import SourceView from './SourceView';

it('uses actual language grammars and keeps multiline syntax balanced on each display row',()=>{
  const text='/* comment\n * continued */\nexport function greet(name: string) { return "hello" + name; }';
  const lines=highlightedLines(text,'src/greet.ts');
  expect(lines[1]).toContain('class="hljs-comment"');
  const host=document.createElement('div');
  lines.forEach((html,index)=>{host.innerHTML=html;expect(host.textContent).toBe(text.split('\n')[index]);expect(host.querySelectorAll('span')).toHaveLength((html.match(/<span /g)??[]).length);});
  expect(lines[2]).toContain('hljs-keyword'); expect(lines[2]).toContain('hljs-title function_'); expect(lines[2]).toContain('hljs-string');
  expect(highlightCode('const view = <Panel title="Test"/>;','App.tsx')).toContain('hljs-name');
  expect(highlightCode('fn main() { let n = 42; }','main.rs')).toContain('hljs-number');
  expect(highlightCode('color: red;','styles.css')).toContain('hljs-attribute');
});
it('escapes source markup and unknown languages instead of creating executable DOM',()=>{
  const source='<img src=x onerror="alert(1)"><script>alert(1)</script>';
  for(const language of ['html','tsx','unknown']) {
    const host=document.createElement('div');host.innerHTML=highlightCode(source,language);
    expect(host.querySelector('script,img')).toBeNull();expect(host.textContent).toBe(source);
  }
});
it('colors source previews without losing line navigation or original copyable text',async()=>{
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<SourceView name="main.ts" text={'// note\nconst value = "text";\nconsole.log(value);'} line={2}/>));
    expect(host.querySelector('[data-line="2"]')?.classList.contains('source-selected')).toBe(true);
    expect(host.querySelector('.hljs-keyword')?.textContent).toBe('const');
    expect(host.querySelector('.hljs-string')?.textContent).toBe('"text"');
    expect(host.querySelector('[data-line="3"] code')?.textContent).toBe('console.log(value);');
  } finally {await act(async()=>root.unmount());host.remove();}
});
