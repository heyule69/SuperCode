import { useCallback, useEffect, useRef, useState } from 'react';
import { call, desktop } from './api';
import type { LinkageConfiguration, LinkageChoice, LinkageCatalog } from './LinkageSetup';
import type { Agent, AgentProfile, Message, RpcEvent, Session } from './types';
import { initialModelSelection } from './modelSelection';

export interface LinkageTask { objective:string; state:string; round:number; steps:{text:string;done:boolean;evidence:string}[]; reason?:string|null; attempts?:string[]; reviewError?:string|null; executionError?:string|null }
export interface LinkageState { configuration:LinkageConfiguration|null; worker:Session|null; messages:Message[]; active:boolean; task?:LinkageTask|null }
const emptyState:LinkageState={configuration:null,worker:null,messages:[],active:false};
export function defaultLinkageChoice(agent:string,profiles:AgentProfile[],officialAgents:string[]):LinkageChoice{
  const selection=initialModelSelection(agent,profiles,officialAgents);
  const profile=profiles.find(profile=>profile.agent===agent&&profile.current)??profiles.find(profile=>profile.agent===agent);
  return {agentId:agent,connectionId:selection?.connectionId??profile?.id??(officialAgents.includes(agent)?'@official':'@local'),modelId:selection?.model??profile?.model??'',effort:selection?.effort??'',...(selection?.serviceTier?{serviceTier:selection.serviceTier}:{})};
}
export function linkageCatalogs(agents:Agent[],profiles:AgentProfile[],officialAgents:string[],choices:LinkageChoice[]):LinkageCatalog[]{
  return agents.map(agent=>{
    const rows=profiles.filter(profile=>profile.agent===agent.id).sort((a,b)=>Number(b.current)-Number(a.current));
    const selected=choices.filter(choice=>choice.agentId===agent.id).map(choice=>choice.connectionId);
    const native=[...new Set([...selected.filter(id=>id==='@local'||id==='@official'),...(officialAgents.includes(agent.id)?['@official']:rows.length?[]:['@local'])])];
    const nativeProfiles:AgentProfile[]=native.filter(id=>!rows.some(profile=>profile.id===id)).map(id=>({id,agent:agent.id,name:id==='@official'?'官方账号':'本机连接',model:null,hasCredential:false,current:false,officialAccount:id==='@official'}));
    const all=[...rows,...nativeProfiles];
    return {agent,live:true,profiles:all,connections:all.map(profile=>({id:profile.id,name:profile.name,models:[...new Set([profile.model,...profile.models??[]].filter((id):id is string=>!!id))].map(id=>({id,name:id}))}))};
  });
}
export function useLinkage(sessionId:string,draftKey:string){
  const [drafts,setDrafts]=useState<Record<string,LinkageConfiguration>>({});
  const [stored,setStored]=useState<{owner:string;state:LinkageState}>({owner:'',state:emptyState});
  const [error,setError]=useState('');
  const owner=useRef(sessionId);owner.current=sessionId;
  const revision=useRef(0);const timer=useRef<ReturnType<typeof setTimeout>|undefined>(undefined);
  const reload=useCallback(async(id=owner.current)=>{
    if(!desktop||!id)return;
    const request=++revision.current;
    try{const state=await call<LinkageState>('get_linkage_state',{sessionId:id});if(owner.current===id&&request===revision.current){setStored({owner:id,state});setError('');}}
    catch(error){if(owner.current===id&&request===revision.current)setError(String(error));}
  },[]);
  useEffect(()=>{revision.current++;setError('');void reload();return()=>{revision.current++;if(timer.current)clearTimeout(timer.current);timer.current=undefined;};},[sessionId,reload]);
  const observe=useCallback((event:RpcEvent)=>{
    if(event.method!=='linkage/updated'||event.params.appSessionId!==owner.current)return;
    if(timer.current)return;
    timer.current=setTimeout(()=>{timer.current=undefined;void reload();},120);
  },[reload]);
  const saveDraft=useCallback((key:string,configuration:LinkageConfiguration)=>{setDrafts(old=>({...old,[key]:configuration}));},[]);
  const persist=useCallback(async(id:string,configuration:LinkageConfiguration)=>{
    const state=await call<LinkageState>('configure_linkage',{sessionId:id,configuration});
    setDrafts(old=>{const next={...old};delete next[id];return next;});
    if(owner.current===id){revision.current++;setStored({owner:id,state});setError('');}
    return state;
  },[]);
  const state=stored.owner===sessionId?stored.state:emptyState;
  const configuration=drafts[sessionId||draftKey]??state.configuration;
  return {state,configuration,enabled:configuration?.mode==='linked',error,reload,observe,saveDraft,persist};
}
