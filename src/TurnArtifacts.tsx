import { memo, useContext, useEffect, useState } from 'react';
import { Check, Download, FolderOpen } from 'lucide-react';
import { call, desktop } from './api';
import { FileIcon } from './FileIcon';
import { MediaContext } from './Media';
import { systemDocument } from './links';
import type { ArtifactFile } from './artifacts';
import './turn-artifacts.css';

function sizeLabel(size:number) {
  return size<1024?`${size} B`:size<1024*1024?`${(size/1024).toFixed(1)} KB`:`${(size/1024/1024).toFixed(1)} MB`;
}

export const TurnArtifacts=memo(function TurnArtifacts({pathsKey,collectionError}:{pathsKey:string;collectionError?:string}) {
  const context=useContext(MediaContext);
  const [result,setResult]=useState<{key:string;files:ArtifactFile[]}>({key:'',files:[]});
  const [error,setError]=useState('');
  const [saved,setSaved]=useState('');
  const [working,setWorking]=useState('');
  const key=JSON.stringify([context.projectId,context.sessionId,pathsKey]);
  useEffect(()=>{
    const paths=JSON.parse(pathsKey) as string[];
    setError('');setSaved('');
    if(!desktop||!paths.length)return;
    let disposed=false;
    void call<ArtifactFile[]>('resolve_artifact_references',{paths,projectId:context.projectId??'',sessionId:context.sessionId||null})
      .then(files=>{if(!disposed)setResult({key,files});})
      .catch(error=>{if(!disposed)setError(String(error));});
    return()=>{disposed=true;};
  },[key,pathsKey,context.projectId,context.sessionId]);
  const files=result.key===key?result.files:[];
  async function action(file:ArtifactFile,kind:'save'|'reveal') {
    if(working)return;
    setWorking(file.path);setError('');setSaved('');
    try {
      const owner={projectId:context.projectId??'',sessionId:context.sessionId||null};
      if(kind==='reveal')await call('open_project_path',{...owner,path:file.path,action:'reveal'});
      else {
        const {save}=await import('@tauri-apps/plugin-dialog');
        const destination=await save({title:'另存最终文件',defaultPath:file.name});
        if(destination){await call('save_artifact_copy',{...owner,path:file.path,destination});setSaved(file.path);}
      }
    } catch(error){setError(String(error));}
    finally {setWorking('');}
  }
  return <>{files.length?<section className="turn-artifacts" aria-label="最终文件">{files.map(file=>{
    const preview=systemDocument(file.path)||/\.(?:[cm]?[jt]sx?|py|rs|go|java|c|cpp|h|cs|rb|sh|ps1|sql|toml|ya?ml|xml|ini|conf)$/i.test(file.name);
    return <div className="artifact-file" key={file.path} data-resource-path={file.path}>
      <button type="button" className="artifact-open" title={file.path} onClick={()=>preview&&context.openFile?context.openFile(file.path):void action(file,'reveal')} disabled={!!working}>
        <span className="artifact-icon"><FileIcon name={file.name} size={26}/></span><span className="artifact-description"><strong>{file.name}</strong><small>{file.name.includes('.')?file.name.split('.').pop()!.toUpperCase():'文件'} · {sizeLabel(file.size)}</small></span>
      </button>
      <button type="button" className="icon-button" title="在文件夹中显示" aria-label={`定位 ${file.name}`} disabled={!!working} onClick={()=>void action(file,'reveal')}><FolderOpen size={16}/></button>
      <button type="button" className="icon-button" title={saved===file.path?'已另存':'另存为'} aria-label={`另存为 ${file.name}`} disabled={!!working} onClick={()=>void action(file,'save')}>{saved===file.path?<Check size={16}/>:<Download size={16}/>}</button>
    </div>;
  })}</section>:null}{error||collectionError?<p className="inline-error" role="status">{error||collectionError}</p>:null}</>;
});
