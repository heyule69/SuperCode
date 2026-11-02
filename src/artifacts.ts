import { describeLink } from './links';
import type { Message } from './types';
import { artifactLink } from './artifactLinks';

export interface ArtifactFile { path:string; name:string; size:number; kind?:'directory' }

export function artifactPathKey(path:string):string {
  const target=describeLink(path).target.replace(/\\/g,'/').replace(/\/$/,'');
  return /^(?:[a-z]:\/|\/\/)/i.test(target)?target.toLowerCase():target;
}

export function turnKnownArtifacts(items:Message[]):ArtifactFile[] {
  const files=items.filter(message=>message.kind==='turnArtifacts').flatMap(message=>{
    const values=message.data?.artifacts;
    return Array.isArray(values)?values.filter((file:unknown):file is ArtifactFile=>!!file&&typeof file==='object'&&typeof (file as ArtifactFile).path==='string'&&typeof (file as ArtifactFile).name==='string'&&Number.isFinite((file as ArtifactFile).size)&&((file as ArtifactFile).size>=0)):[];
  });
  return [...new Map(files.map(file=>[artifactPathKey(file.path),file])).values()].slice(0,64);
}

export function referencedArtifacts(text:string):string[] {
  const prose=text.replace(/```[^]*?(?:```|$)|~~~[^]*?(?:~~~|$)/g,'');
  const paths:string[]=[];
  for(const match of prose.matchAll(/(?<!!)\[[^\]]*\]\(\s*(?:<([^>]+)>|([^\s)]+))(?:\s+"[^"]*")?\s*\)|`([^`\n]+)`/g)) {
    const code=match[3];
    // Inline paths can describe configuration or implementation details. Only
    // standalone paths or delivery wording under outputs count as output claims.
    if(code){
      if(!/(?:^|[\\/])outputs[\\/]/i.test(code))continue;
      const start=prose.lastIndexOf('\n',match.index)+1,end=prose.indexOf('\n',match.index);
      const line=prose.slice(start,end<0?prose.length:end);
      const context=line.replace(match[0],'');
      if(/旧版|旧文件|占位|示例|删除|移除|未生成|尚未|将来|placeholder|not generated/i.test(context))continue;
      if(!/^(?:(?:[-+*]|\d+[.)])\s+)?`[^`\n]+`$/.test(line.trim())&&!/已(?:生成|保存|导出|完成)|文件(?:位置|路径)|产物|成果|交付|下载|保存到|生成到|输出到|附件|saved|generated|output|deliverable/i.test(context))continue;
    }
    const link=describeLink(code??match[1]??match[2]);
    if(link?.kind==='file'&&!link.line&&!artifactLink(match[1]??match[2]??code))paths.push(link.target);
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
