import { memo, useContext, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { Check, ChevronDown, ChevronLeft, ChevronRight, Download, ExternalLink, Files, Film, FolderOpen, Globe, Images, Music2, RefreshCw } from 'lucide-react';
import { call, desktop } from './api';
import { FileIcon } from './FileIcon';
import { MediaContext } from './Media';
import { systemDocument } from './links';
import { mediaKind } from './mediaSources';
import { artifactPathKey, type ArtifactFile } from './artifacts';
import { artifactLink, type ArtifactLink } from './artifactLinks';
import { artifactFileLabel, artifactSize as sizeLabel } from './artifactPresentation';
import { CopyButton } from './CopyButton';
import './turn-artifacts.css';

const PAGE_SIZE=8;
const groupKinds=['link','directory','file','image','video','audio'] as const;
const groupLabels={link:{label:'链接',unit:'个',Icon:Globe},directory:{label:'文件夹',unit:'个',Icon:FolderOpen},file:{label:'文件',unit:'个',Icon:Files},image:{label:'图片',unit:'张',Icon:Images},video:{label:'视频',unit:'个',Icon:Film},audio:{label:'音频',unit:'个',Icon:Music2}};
type ArtifactKind=typeof groupKinds[number];
type ArtifactEntry={kind:'link';link:ArtifactLink}|{kind:Exclude<ArtifactKind,'link'>;file:ArtifactFile;missing?:boolean};
const entryKey=(entry:ArtifactEntry)=>entry.kind==='link'?`url:${entry.link.url}`:artifactPathKey(entry.file.path);
const fileEntry=(file:ArtifactFile,missing=false):ArtifactEntry=>({kind:file.kind==='directory'?'directory':mediaKind(file.path)??'file',file,missing});
const byteTotal=(entries:ArtifactEntry[])=>entries.reduce((sum,entry)=>sum+(entry.kind!=='link'&&!entry.missing&&entry.kind!=='directory'?entry.file.size:0),0);
const onlyFiles=(entries:ArtifactEntry[])=>entries.every(entry=>entry.kind!=='link'&&entry.kind!=='directory'&&!entry.missing);

function ArtifactGroup({kind,entries,renderEntry}:{kind:ArtifactKind;entries:ArtifactEntry[];renderEntry:(entry:ArtifactEntry)=>ReactNode}) {
  const [expanded,setExpanded]=useState(false);
  const [page,setPage]=useState(0);
  const id=useId();
  const {label,unit,Icon}=groupLabels[kind];
  const direct=['file','link','directory'].includes(kind);
  const collapsible=!direct||entries.length>3;
  const lastPage=Math.ceil(entries.length/PAGE_SIZE)-1;
  const current=Math.min(page,lastPage);
  const start=expanded?current*PAGE_SIZE:0;
  const visible=expanded?entries.slice(start,start+PAGE_SIZE):direct?entries.slice(0,3):[];
  const summary=<><Icon size={16}/><strong>{label}</strong><span className="artifact-group-count">{entries.length} {unit}</span><span className="artifact-group-size">{onlyFiles(entries)?sizeLabel(byteTotal(entries)):null}</span></>;
  return <div className="artifact-group" data-artifact-kind={kind}>
    {collapsible?<button type="button" className="artifact-group-summary" aria-label={`${expanded?'收起':'展开'}${label}`} aria-expanded={expanded} aria-controls={id} onClick={()=>setExpanded(value=>!value)}>
      {summary}<span className="artifact-group-action">{expanded?'收起':direct?'查看全部':'展开'}</span><ChevronDown size={14} className={expanded?'expanded':''}/>
    </button>:null}
    <div className="artifact-group-files" id={id} hidden={!visible.length}>{visible.map(renderEntry)}</div>
    {expanded&&lastPage>0?<nav className="artifact-pagination" aria-label={`${label}分页`}>
      <button type="button" className="icon-button" aria-label={`上一页${label}`} disabled={current===0} onClick={()=>setPage(current-1)}><ChevronLeft size={15}/></button>
      <span>{start+1}–{Math.min(start+PAGE_SIZE,entries.length)} / {entries.length}</span>
      <button type="button" className="icon-button" aria-label={`下一页${label}`} disabled={current===lastPage} onClick={()=>setPage(current+1)}><ChevronRight size={15}/></button>
    </nav>:null}
  </div>;
}

function ArtifactCollection({entries,renderEntry}:{entries:ArtifactEntry[];renderEntry:(entry:ArtifactEntry)=>ReactNode}) {
  const groups=useMemo(()=>{
    const grouped=new Map<ArtifactKind,ArtifactEntry[]>();
    for(const entry of entries){const group=grouped.get(entry.kind)??[];group.push(entry);grouped.set(entry.kind,group);}
    return groupKinds.flatMap(kind=>{const group=grouped.get(kind);return group?[{kind,entries:group}]:[];});
  },[entries]);
  const pureFiles=onlyFiles(entries);
  return <section className="turn-artifacts artifact-collection" aria-label={pureFiles?'最终文件':'交付成果'}>
    <header className="artifact-collection-heading"><Files size={16}/><strong>{pureFiles?'本轮文件':'本轮产物'}</strong><span>{entries.length} {pureFiles?'个文件':'项'}</span>{pureFiles?<small>{sizeLabel(byteTotal(entries))}</small>:null}</header>
    {groups.map(group=><ArtifactGroup key={group.kind} kind={group.kind} entries={group.entries} renderEntry={renderEntry}/>)}
  </section>;
}

interface Props {pathsKey:string;linksKey?:string;knownKey?:string;collectionError?:string}
function arrayValue(value:string):unknown[] {try{const parsed=JSON.parse(value);return Array.isArray(parsed)?parsed.slice(0,64):[];}catch{return [];}}
function filesValue(values:unknown[]):ArtifactFile[] {
  return values.filter((value):value is ArtifactFile=>!!value&&typeof value==='object'&&typeof (value as ArtifactFile).path==='string'&&typeof (value as ArtifactFile).name==='string'&&Number.isFinite((value as ArtifactFile).size)&&(value as ArtifactFile).size>=0);
}

export const TurnArtifacts=memo(function TurnArtifacts(props:Props) {
  const context=useContext(MediaContext);
  return <ArtifactResults key={JSON.stringify([context.projectId,context.sessionId,props.pathsKey,props.linksKey,props.knownKey])} {...props}/>;
});

function ArtifactResults({pathsKey,linksKey='[]',knownKey='[]',collectionError}:Props) {
  const context=useContext(MediaContext);
  const paths=useMemo(()=>arrayValue(pathsKey).filter((value):value is string=>typeof value==='string'),[pathsKey]);
  const links=useMemo(()=>[...new Map(arrayValue(linksKey).flatMap(value=>{
    if(!value||typeof value!=='object')return [];
    const input=value as Partial<ArtifactLink>,link=artifactLink(typeof input.url==='string'?input.url:'',typeof input.title==='string'?input.title:'');
    return link?[link]:[];
  }).map(link=>[link.url,link])).values()],[linksKey]);
  const known=useMemo(()=>filesValue(arrayValue(knownKey)),[knownKey]);
  const [result,setResult]=useState<{status:'loading'|'ready'|'error';files:ArtifactFile[];error?:string}>({status:desktop&&paths.length?'loading':'ready',files:[]});
  const [attempt,setAttempt]=useState(0);
  const [error,setError]=useState('');
  const [saved,setSaved]=useState('');
  const [working,setWorking]=useState('');
  const alive=useRef(true),locked=useRef(false);
  const retryAction=useRef<(()=>Promise<void>)|null>(null);
  useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
  useEffect(()=>{
    if(!desktop||!paths.length)return;
    let disposed=false;
    setResult({status:'loading',files:[]});
    void call<ArtifactFile[]>('resolve_artifact_references',{paths,projectId:context.projectId??'',sessionId:context.sessionId||null})
      .then(files=>{if(!disposed)setResult({status:'ready',files:filesValue(Array.isArray(files)?files:[])});})
      .catch(error=>{if(!disposed)setResult({status:'error',files:[],error:String(error)});});
    return()=>{disposed=true;};
  },[paths,attempt,context.projectId,context.sessionId]);
  const missing=result.status==='ready'?known.filter(file=>!result.files.some(current=>artifactPathKey(current.path)===artifactPathKey(file.path))):[];
  const entries:ArtifactEntry[]=[...links.map((link):ArtifactEntry=>({kind:'link',link})),...result.files.map(file=>fileEntry(file)),...missing.map(file=>fileEntry(file,true))];
  async function perform(id:string,operation:()=>Promise<void>):Promise<void> {
    if(locked.current)return;
    locked.current=true;setWorking(id);setError('');setSaved('');
    retryAction.current=()=>perform(id,operation);
    try{await operation();}catch(error){if(alive.current)setError(String(error));}
    finally{locked.current=false;if(alive.current)setWorking('');}
  }
  async function action(file:ArtifactFile,kind:'save'|'reveal'|'open') {
    await perform(file.path,async()=>{
      const owner={projectId:context.projectId??'',sessionId:context.sessionId||null};
      if(kind!=='save')await call('open_project_path',{...owner,path:file.path,action:kind});
      else{
        const {save}=await import('@tauri-apps/plugin-dialog');
        const destination=await save({title:'另存最终文件',defaultPath:file.name});
        if(destination){await call('save_artifact_copy',{...owner,path:file.path,destination});if(alive.current)setSaved(file.path);}
      }
    });
  }
  function renderEntry(entry:ArtifactEntry) {
    if(entry.kind==='link'){
      const {link}=entry;
      return <div className="artifact-file artifact-link" key={entryKey(entry)} data-artifact-url={link.url}>
        <a className="artifact-open" href={link.url} target="_blank" rel="noopener noreferrer" title={link.display} aria-disabled={!!working} onClick={event=>{
          if(working){event.preventDefault();return;}
          if(desktop){event.preventDefault();void perform(link.url,()=>call('open_external_link',{url:link.url}));}
        }}>
          <span className="artifact-icon artifact-web-icon">{link.kind==='download'?<Download size={21}/>:<Globe size={21}/>}</span>
          <span className="artifact-description"><span className="artifact-link-kind">{link.kind==='download'?'下载链接':link.local?'本地预览':'网页'}</span><strong>{link.title}</strong><small>{link.display}</small></span><ExternalLink size={15} className="artifact-external-icon"/>
        </a>
        <CopyButton text={link.url} label={`复制链接 ${link.title}`} iconOnly/>
      </div>;
    }
    const {file}=entry,directory=entry.kind==='directory',html=/\.html?$/i.test(file.name);
    const preview=systemDocument(file.path)||/\.(?:[cm]?[jt]sx?|py|rs|go|java|c|cpp|h|cs|rb|sh|ps1|sql|toml|ya?ml|xml|ini|conf|json|css|scss)$/i.test(file.name);
    const duplicates=result.files.filter(other=>other.name===file.name).length>1;
    const name=duplicates?file.path.replace(/\\/g,'/'):file.name;
    const metadata=entry.missing?'文件已移动或删除':directory?'文件夹':`${artifactFileLabel(file)} · ${file.name.includes('.')?file.name.split('.').pop()!.toUpperCase():'无扩展名'} · ${sizeLabel(file.size)}${file.size===0?' · 空文件':''}`;
    return <div className={`artifact-file${entry.missing?' artifact-unavailable':''}`} key={entryKey(entry)} data-resource-path={file.path}>
      <button type="button" className="artifact-open" title={file.path} onClick={()=>directory||html?void action(file,'open'):preview&&context.openFile?context.openFile(file.path):void action(file,'reveal')} disabled={!!working||entry.missing}>
        <span className="artifact-icon"><FileIcon name={file.name} directory={directory} size={22}/></span><span className="artifact-description"><strong>{name}</strong><small>{metadata}</small></span>
      </button>
      {entry.missing?<button type="button" className="icon-button" aria-label={`重新检查 ${file.name}`} onClick={()=>setAttempt(value=>value+1)}><RefreshCw size={16}/></button>:<>
        <button type="button" className="icon-button" title="在文件夹中显示" aria-label={`定位 ${file.name}`} disabled={!!working} onClick={()=>void action(file,'reveal')}><FolderOpen size={16}/></button>
        {directory?<CopyButton text={file.path} label={`复制路径 ${file.name}`} iconOnly/>:<button type="button" className="icon-button" title={saved===file.path?'已另存':'另存为'} aria-label={`另存为 ${file.name}`} disabled={!!working} onClick={()=>void action(file,'save')}>{saved===file.path?<Check size={16}/>:<Download size={16}/>}</button>}
      </>}
    </div>;
  }
  return <>{entries.length>3?<ArtifactCollection entries={entries} renderEntry={renderEntry}/>:entries.length?<section className="turn-artifacts" aria-label={onlyFiles(entries)?'最终文件':'交付成果'}>{entries.map(renderEntry)}</section>:null}
    {result.status==='loading'?<p className="artifact-feedback" role="status">正在检查文件…</p>:null}
    {result.status==='error'?<p className="artifact-feedback inline-error" role="status"><span>{result.error}</span><button type="button" className="text-button" onClick={()=>setAttempt(value=>value+1)}>重新检查产物</button></p>:null}
    {missing.length?<p className="artifact-feedback" role="status">有 {missing.length} 个历史文件已移动或删除</p>:null}
    {error?<p className="artifact-feedback inline-error" role="status"><span>{error}</span><button type="button" className="text-button" disabled={!!working} onClick={()=>void retryAction.current?.()}>重试操作</button></p>:collectionError?<p className="inline-error" role="status">{collectionError}</p>:null}
  </>;
}
