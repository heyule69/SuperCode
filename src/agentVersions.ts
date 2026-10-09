export interface AgentRelease { id:string; latestVersion:string|null; checkedAt:number|null; error:string|null }
function version(value:string|undefined|null) {
  const match=value?.match(/(?:^|\s)v?(\d+)\.(\d+)\.(\d+)(?:-([\da-zA-Z.-]+))?(?:\+[\da-zA-Z.-]+)?(?=\s|$)/);
  return match?{parts:match.slice(1,4).map(part=>BigInt(part)),prerelease:!!match[4]}:null;
}
export function releaseState(local:string|undefined|null,latest:string|undefined|null):'update'|'current'|'newer'|'unknown' {
  const a=version(local),b=version(latest);
  if(!a||!b)return 'unknown';
  for(let i=0;i<3;i++){if(a.parts[i]<b.parts[i])return 'update';if(a.parts[i]>b.parts[i])return 'newer';}
  return a.prerelease&&!b.prerelease?'update':'current';
}
