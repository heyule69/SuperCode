import { describeLink } from './links';
import type { Message } from './types';

export interface ArtifactFile { path:string; name:string; size:number }

export function referencedArtifacts(text:string):string[] {
  const prose=text.replace(/```[^]*?(?:```|$)|~~~[^]*?(?:~~~|$)/g,'');
  const paths:string[]=[];
  for(const match of prose.matchAll(/(?<!!)\[[^\]]*\]\(\s*(?:<([^>]+)>|([^\s)]+))(?:\s+"[^"]*")?\s*\)|`([^`\n]+)`/g)) {
    const code=match[3];
    // Inline paths can describe configuration or implementation details. Only
    // the designated delivery directory makes an inline path an output claim.
    const link=code?(/(?:^|[\\/])outputs[\\/]/i.test(code)?describeLink(code):null):describeLink(match[1]??match[2]);
    if(link?.kind==='file'&&!link.line)paths.push(link.target);
  }
  return [...new Set(paths)];
}

export function turnArtifactPaths(items:Message[],final:Message[]):string[] {
  const generated=items.filter(message=>message.kind==='turnArtifacts').flatMap(message=>{
    const files=message.data?.artifacts;
    return Array.isArray(files)?files.flatMap(file=>typeof file?.path==='string'?[file.path]:[]):[];
  });
  return [...new Set([...generated,...final.flatMap(message=>referencedArtifacts(message.text))])].slice(0,64);
}
