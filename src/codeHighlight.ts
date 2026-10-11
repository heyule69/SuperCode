import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import css from 'highlight.js/lib/languages/css';
import json from 'highlight.js/lib/languages/json';
import rust from 'highlight.js/lib/languages/rust';
import python from 'highlight.js/lib/languages/python';
import bash from 'highlight.js/lib/languages/bash';
import yaml from 'highlight.js/lib/languages/yaml';

// This module is only pulled in by lazy code and review views.
for (const [name, grammar] of Object.entries({ javascript, typescript, xml, css, json, rust, python, bash, yaml })) hljs.registerLanguage(name, grammar);
const languages: Record<string,string> = { js:'javascript',jsx:'javascript',mjs:'javascript',cjs:'javascript',ts:'typescript',tsx:'typescript',html:'xml',htm:'xml',svg:'xml',vue:'xml',css:'css',json:'json',rs:'rust',py:'python',sh:'bash',bash:'bash',yml:'yaml',yaml:'yaml' };
const escape = (text:string) => text.replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#x27;'}[character]!));
export function highlightCode(text:string, pathOrLanguage='') {
  const hint=pathOrLanguage.split('.').pop()?.toLowerCase()??'';
  const language=languages[hint]??(hljs.getLanguage(hint)?hint:'');
  if (!language) return escape(text);
  try { return hljs.highlight(text,{language,ignoreIllegals:true}).value; } catch { return escape(text); }
}
export function highlightedLines(text:string, pathOrLanguage='') {
  const spans:string[]=[];
  // Highlight a block at once, then balance generated spans for each displayed
  // row. Multiline comments and strings keep their scope across line numbers.
  return highlightCode(text,pathOrLanguage).split('\n').map(fragment=>{
    const prefix=spans.join('');
    for (const match of fragment.matchAll(/<span class="[\w ._-]+">|<\/span>/g)) {
      if (match[0]==='</span>') spans.pop(); else spans.push(match[0]);
    }
    return prefix+fragment+'</span>'.repeat(spans.length);
  });
}
