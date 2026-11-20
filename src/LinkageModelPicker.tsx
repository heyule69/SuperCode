import { useEffect, useMemo, useRef, useState } from 'react';
import { call } from './api';
import { ModelMenu } from './ModelMenu';
import type { AgentProfile, ModelCatalog, ModelSource } from './types';
import type { LinkageCatalog, LinkageChoice } from './LinkageSetup';

// Use the existing model picker, including its search, connection groups,
// keyboard controls and reasoning settings. The linkage UI only binds a role.
export function LinkageModelPicker({ value, catalog, onChange, manage, disabled = false }: {
  value: LinkageChoice; catalog?: LinkageCatalog; onChange: (value: LinkageChoice) => void;
  manage: (agentId: string) => void; disabled?: boolean;
}) {
  const [remote,setRemote]=useState<{key:string;catalog?:ModelCatalog;error?:string}>();
  const [revision,setRevision]=useState(0);
  const latest=useRef({value,onChange});latest.current={value,onChange};
  const key=`${value.agentId}:${value.connectionId}`;
  useEffect(()=>{
    if(!catalog?.live||!value.connectionId)return;
    let disposed=false;setRemote({key});
    void call<ModelCatalog>('list_models',{agent:value.agentId,connectionId:value.connectionId,forceRefresh:revision>0}).then(result=>{
      if(disposed)return;setRemote({key,catalog:result});
      const current=latest.current;
      if(!current.value.modelId&&`${current.value.agentId}:${current.value.connectionId}`===key){
        const model=result.data.find(model=>model.isDefault)??result.data[0];
        if(model)current.onChange({...current.value,modelId:model.model,effort:model.defaultReasoningEffort??''});
      }
    }).catch(error=>{if(!disposed)setRemote({key,error:String(error)});});
    return()=>{disposed=true;};
  },[catalog?.live,key,value.agentId,value.connectionId,revision]);
  const fixtureProfiles = useMemo<AgentProfile[]>(() => (catalog?.connections ?? []).map(connection => ({
    id: connection.id, name: connection.name, agent: value.agentId,
    model: connection.models[0]?.id ?? null, models: connection.models.map(model => model.id),
    hasCredential: true, current: connection.id === value.connectionId, providerId: 'custom',
    modelSource: { providerId: 'custom', providerName: connection.name, connectionName: connection.name, mark: '' },
  })), [catalog, value.agentId, value.connectionId]);
  const profiles=catalog?.profiles??fixtureProfiles;
  const connection = catalog?.connections.find(item => item.id === value.connectionId);
  const models = useMemo(() => (connection?.models ?? []).map((model, index) => ({ id: model.id, model: model.id, displayName: model.name, isDefault: index === 0 })), [connection]);
  const activeProfile = profiles.find(profile => profile.id === value.connectionId);
  const loaded=remote?.key===key?remote:undefined;
  const source: ModelSource | undefined = catalog?.live?loaded?.catalog?.source:activeProfile?.modelSource ? { ...activeProfile.modelSource, available: !catalog?.error } : undefined;
  return <ModelMenu value={value.modelId} models={catalog?.live?loaded?.catalog?.data??models:models} catalogLoaded={!catalog?.live||!!loaded?.catalog} profiles={profiles} activeProfile={activeProfile} source={source}
    connectionId={value.connectionId} busy={disabled} loading={catalog?.live?!loaded?.catalog&&!loaded?.error:!!catalog?.loading} catalogWarning={loaded?.error??loaded?.catalog?.warning??catalog?.error} agent={value.agentId}
    choose={async option => onChange({ ...value, connectionId: option.connectionId, modelId: option.model, effort: '', serviceTier: undefined })}
    reload={() => catalog?.live?setRevision(value=>value+1):manage(value.agentId)} manage={() => manage(value.agentId)} effort={value.effort ?? ''}
    setEffort={effort => onChange({ ...value, effort })} serviceTier={value.serviceTier} setServiceTier={serviceTier => onChange({ ...value, serviceTier })} hasConversation={false}/>;
}
