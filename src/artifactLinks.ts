import { mediaKind } from './mediaSources';
import type { Message } from './types';

export interface ArtifactLink { url:string; title:string; kind:'website'|'download'; local:boolean; display:string }
const delivered=/(已(?:经)?(?:成功)?(?:发布|部署|上线|生成)|网址|网页链接|网站地址|项目地址|服务地址|接口地址|访问地址|打开(?:网站|网页|页面|应用|链接)|访问(?:网站|网页|页面)|查看(?:成品|成果)|源码|代码仓库|项目仓库|仓库地址|预览|在线(?:演示|体验|查看|使用)|下载|交付|最终(?:链接|地址|成果)|deployed|published|live\s+demo|preview|download|deliverable|repository)/i;
const references=/(参考|引用|来源|文档|教程|依赖|官网|reference|source|documentation|\bdocs\b)/i;
const citation=/(参考|引用|来源|依赖|官网|\breferences?\b|\bsources?\b)/i;
const hypothetical=/(占位|示例地址|尚未|未部署|未发布|还没|将来|(?:不是|并非|不作为|不属于).{0,8}(?:交付|产物|成果|发布|部署)|placeholder|not deployed|example URL)/i;
const completed=/(已(?:经)?(?:成功)?(?:发布|部署|上线|生成)|deployed|published)/i;

export function artifactLink(source:string,title=''):ArtifactLink|null {
  if(source.length>8192||/[\u0000-\u001f\u007f]/.test(source))return null;
  try {
    const address=/^(?:localhost|127(?:\.\d{1,3}){3}|\[::1\]):\d+(?:\/|$)/i.test(source)?`http://${source}`:source;
    const url=new URL(address);
    if(!['http:','https:'].includes(url.protocol)||!url.hostname||url.username||url.password)return null;
    const label=title.replace(/[*_`]/g,'').replace(/[\u0000-\u001f\u007f]/g,'').trim().slice(0,160);
    const local=url.hostname==='localhost'||url.hostname.endsWith('.localhost')||/^127(?:\.\d{1,3}){3}$/.test(url.hostname)||['[::1]','0.0.0.0'].includes(url.hostname);
    const download=/\.(?:zip|7z|rar|tar|gz|exe|msi|msix|appx|dmg|apk|pdf|docx?|xlsx?|pptx?|csv|epub)$/i.test(url.pathname)||/下载|download/i.test(label);
    let path=url.pathname;try{path=decodeURI(path);}catch{/* Keep malformed escapes visible as supplied. */}
    return {url:url.href,title:label&&!/^https?:/i.test(label)?label:url.hostname,kind:download?'download':'website',local,display:`${url.host}${path==='/'?'':path}`};
  }catch{return null;}
}

// Only final delivery references become cards. Documentation, examples and
// embedded media keep their existing text/preview presentation.
export function referencedArtifactLinks(text:string):ArtifactLink[] {
  const prose=text.slice(0,256*1024).replace(/```[^]*?(?:```|$)|~~~[^]*?(?:~~~|$)|<!--[^]*?(?:-->|$)/g,match=>match.replace(/[^\n]/g,' '));
  const result=new Map<string,{link:ArtifactLink;start:number}>();
  function add(source:string,title:string,start:number,end:number) {
    const link=artifactLink(source.replace(/&(?:amp|#0*38|#x0*26);/gi,'&').replace(/\\([()[\]\\])/g,'$1'),title);
    if(!link||mediaKind(link.url))return;
    const lineStart=prose.lastIndexOf('\n',start)+1;
    const next=prose.indexOf('\n',end),lineEnd=next<0?prose.length:next;
    const before=prose.slice(lineStart,start),after=prose.slice(end,lineEnd);
    const previous=prose.slice(Math.max(0,lineStart-200),lineStart).trim().split('\n').at(-1)??'';
    const context=`${before}${title}${after}`;
    const ready=completed.test(before)||completed.test(previous);
    if(/^\d+$/.test(title)||hypothetical.test(context))return;
    if(citation.test(context)&&!completed.test(before)&&!/下载|download/i.test(before+title))return;
    if(references.test(context)&&!ready)return;
    const standalone=!before.replace(/^\s*(?:[-*+]|\d+[.)])?\s*/,'').trim().replace(/^<$/,'')&&!after.trim().replace(/^>$/,'');
    if(!delivered.test(context)&&!(standalone&&!references.test(previous))&&!(delivered.test(previous)&&(!references.test(previous)||ready)))return;
    const existing=result.get(link.url);
    if(existing&&existing.start<=start)return;
    if(!existing&&result.size>=64){const last=[...result.values()].reduce((a,b)=>a.start>b.start?a:b);if(start>=last.start)return;result.delete(last.link.url);}
    result.set(link.url,{link,start});
  }
  let masked=prose.replace(/`([^`\n]+)`/g,(match,source:string,index:number)=>{
    if(/^(?:https?:\/\/|localhost:|127(?:\.\d{1,3}){3}:|\[::1\]:)\S+$/i.test(source))add(source,'',index,index+match.length);
    return ' '.repeat(match.length);
  });
  const links=/(!?)\[([^\]\n]*)\]\(\s*/g;
  const spans:{start:number;end:number}[]=[];
  const definitions=new Map<string,string>();
  for(const match of masked.matchAll(/^[ \t]{0,3}\[([^\]\n]+)\]:[ \t]*(?:<([^>\n]+)>|(\S+))(?:[ \t]+["'][^"'\n]*["'])?[ \t]*$/gm)) {
    if(definitions.size<256)definitions.set(match[1].trim().toLowerCase(),match[2]??match[3]);
    spans.push({start:match.index,end:match.index+match[0].length});
  }
  for(const match of masked.matchAll(links)) {
    let cursor=match.index+match[0].length,source='',end=-1;
    if(masked[cursor]==='<') {
      const close=masked.indexOf('>',cursor+1);
      if(close<0)continue;
      source=masked.slice(cursor+1,close);cursor=close+1;
    }else{
      const begin=cursor;let depth=0;
      while(cursor<masked.length&&cursor-begin<=8192) {
        const character=masked[cursor];
        if(/\s/.test(character)||character===')'&&depth===0)break;
        if(character==='(')depth++;
        if(character===')')depth--;
        cursor++;
      }
      source=masked.slice(begin,cursor);
    }
    const close=/^\s*(?:(?:"[^"\n]*"|'[^'\n]*')\s*)?\)/.exec(masked.slice(cursor,cursor+2048));
    if(close)end=cursor+close[0].length;
    if(end<0)continue;
    spans.push({start:match.index,end});
    if(!match[1])add(source,match[2],match.index,end);
  }
  for(const match of masked.matchAll(/(!?)\[([^\]\n]+)\]\[([^\]\n]*)\]/g)) {
    const source=definitions.get((match[3]||match[2]).trim().toLowerCase());
    if(!source)continue;
    spans.push({start:match.index,end:match.index+match[0].length});
    if(!match[1])add(source,match[2],match.index,match.index+match[0].length);
  }
  for(const span of spans.reverse())masked=masked.slice(0,span.start)+' '.repeat(span.end-span.start)+masked.slice(span.end);
  for(const match of masked.matchAll(/(?:https?:\/\/[^\s<>"'`，。；！？、]+|(?:localhost|127(?:\.\d{1,3}){3}|\[::1\]):\d+(?:\/[^\s<>"'`，。；！？、]*)?)/gi)) {
    let source=match[0].replace(/[.,;!?】》」』]+$/,'');
    while(source.endsWith(')')&&(source.match(/\)/g)?.length??0)>(source.match(/\(/g)?.length??0))source=source.slice(0,-1);
    add(source,'',match.index,match.index+match[0].length);
  }
  return [...result.values()].sort((a,b)=>a.start-b.start).map(item=>item.link);
}

export function turnArtifactLinks(final:Message[]):ArtifactLink[] {
  return [...new Map(final.flatMap(message=>referencedArtifactLinks(message.text)).map(link=>[link.url,link])).values()].slice(0,64);
}
