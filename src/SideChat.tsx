import { lazy, Suspense, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ArrowUp, MessageSquare, Square, X } from 'lucide-react';
import { call, desktop } from './api';
import { useSideChatRecovery } from './useSideChatRecovery';
import { useStableCallback } from './useStableCallback';
import { ComposerTextarea } from './ComposerTextarea';
import { AddMenu, AttachmentStrip, ComposerSkills, ModelMenu, PermissionMenu } from './ComposerMenus';
import { AgentIcon } from './AgentIcon';
import { clipboardImages, imageBase64, imageBytes, MAX_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES, mergeAttachments, validateImageFile, type Attachment } from './attachments';
import type { ModelOption } from './modelPicker';
import type { AgentProfile, ModelCatalog, ModelSource, PermissionMode } from './types';
import { compatiblePermission, loadExecutionPermissionChoices, planExecutionPermission } from './permissions';

const emptyProfiles: AgentProfile[] = [];
const agentNames: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', opencode: 'OpenCode', pi: 'Pi' };
import { createChatSession } from './chatSession';
import { isProviderRequired, requireChatConnection } from './chatConnection';
import { planFollowup, type PlanDecision } from './planPresentation';
import { Conversation } from './Conversation';
import { PlanDisplayContext } from './PlanDisplayContext';
import { FollowupQueue, useFollowups, type FollowupPayload } from './Followups';
import { useFollowupEditor } from './useFollowupEditor';
import { isActive, type Message, type Session, type RpcEvent } from './types';
const RequestCard=lazy(()=>import('./RequestCard'));
const QuestionDock=lazy(()=>import('./QuestionDock'));
export interface SideDraft {key:string;text:string;payload?:FollowupPayload;sourceId?:string;projectId?:string;agent?:string;connectionId?:string}
interface PendingConnection {agent:string;id:string}
export default function SideChat({draft,projectId,agent,connectionId,defaults,close,opened,openFile,showDiff,resizeHandle,providerRequired,manageProviders,pendingConnectionChanged,profiles=emptyProfiles,catalog,enterSend=true,hidden=false,embedded=false}:{draft:SideDraft;projectId:string;agent:string;connectionId:string;defaults:FollowupPayload;close:()=>void;opened:(session:Session)=>void;openFile:(path:string,line?:number,workspaceSessionId?:string)=>void;resizeHandle?:ReactNode;showDiff:(name:string,text:string)=>void;providerRequired:(agent:string,draft:SideDraft)=>void;manageProviders?:(agent:string,draft:SideDraft)=>void;pendingConnectionChanged?:(connection:PendingConnection|null)=>void;profiles?:AgentProfile[];catalog?:ModelCatalog;enterSend?:boolean;hidden?:boolean;embedded?:boolean}){
  const [input,setInput]=useState(draft.text);const [session,setSession]=useState<Session>();const [error,setError]=useState('');const [sending,setSending]=useState(false);const [stopping,setStopping]=useState(false);
  const parentPlanDisplay=useContext(PlanDisplayContext);
  const [planDock,setPlanDock]=useState<HTMLDivElement|null>(null);
  const planDisplay=useMemo(()=>({openPlan:parentPlanDisplay.openPlan,decisionContainer:planDock}),[parentPlanDisplay.openPlan,planDock]);
  const current=useRef(session);current.current=session;const area=useRef<HTMLDivElement>(null);const stickToBottom=useRef(true);
  const [payload,setPayload]=useState(draft.payload??defaults);
  const [initialExecutionPermission]=useState(()=>loadExecutionPermissionChoices({[agent]:compatiblePermission((draft.payload??defaults).permissionMode,agent)})[agent]??'ask');
  const executionPermission=useRef(initialExecutionPermission);
  const options=useRef(payload);options.current=payload;const source=useRef(draft.sourceId);
  const inputRef=useRef(input);inputRef.current=input;
  const [selection,setSelection]=useState(draft.connectionId??connectionId);const [models,setModels]=useState<ModelCatalog>(catalog??{data:[]});
  const [loadingModels,setLoadingModels]=useState(false);const [configuring,updateConfiguring]=useState(false);const configuringLock=useRef(false);const setConfiguring=useCallback((value:boolean)=>{configuringLock.current=value;updateConfiguring(value);},[]);const [importing,setImporting]=useState(false);
  const textarea=useRef<HTMLTextAreaElement>(null);const modelRequest=useRef(0);const sendLock=useRef(false);
  const pendingConnections=useRef(new Map<symbol,PendingConnection>());
  const pendingChanged=useRef(pendingConnectionChanged);pendingChanged.current=pendingConnectionChanged;
  const attachmentJobs=useRef(Promise.resolve());const pendingImports=useRef(0);
  const hasDraft=!!input.trim()||payload.attachments.length>0;const busy=sending||isActive(session?.status);
  const agentName=agentNames[agent]??agent;
  function connectionRoute(s=current.current):PendingConnection{return {agent:s?.agent??agent,id:s?.connectionId??selection};}
  function beginPendingConnection(route=connectionRoute()){
    const token=Symbol();pendingConnections.current.set(token,route);pendingChanged.current?.(route);
    return {active:()=>pendingConnections.current.has(token),finish:()=>{
      if(!pendingConnections.current.delete(token))return;
      const remaining=[...pendingConnections.current.values()].at(-1);pendingChanged.current?.(remaining??null);
    }};
  }
  async function withPendingConnection(action:()=>Promise<void>){const operation=beginPendingConnection();try{await action();}finally{operation.finish();}}
  useEffect(()=>()=>{if(pendingConnections.current.size){pendingConnections.current.clear();pendingChanged.current?.(null);}},[]);
  const { messages, requests, refresh, resolve: resolveRequest } = useSideChatRecovery(current, setSession, report);
  const openSessionFile = useCallback((path:string,line?:number)=>openFile(path,line,session?.id),[openFile,session?.id]);
  function savedDraft():SideDraft{return {...draft,text:inputRef.current,connectionId:selection,payload:options.current,sourceId:source.current};}
  function updateOptions(next:FollowupPayload){options.current=next;setPayload(next);}
  function permission(mode:PermissionMode){
    if(mode==='read'&&options.current.permissionMode!=='read')executionPermission.current=compatiblePermission(options.current.permissionMode,agent);
    else if(mode!=='read')executionPermission.current=compatiblePermission(mode,agent);
    const remembered=loadExecutionPermissionChoices({});remembered[agent]=executionPermission.current;
    localStorage.setItem('supercode.execution-permissions.v1',JSON.stringify(remembered));
    updateOptions({...options.current,permissionMode:mode,readOnly:mode==='read'});
  }
  function removeAttachment(path:string){updateOptions({...options.current,attachments:options.current.attachments.filter(a=>a.path!==path)});textarea.current?.focus();}
  useEffect(()=>{if(!current.current&&!sendLock.current&&draft.connectionId)setSelection(draft.connectionId);},[draft.connectionId]);
  useEffect(()=>{
    let disposed=false;const revision=++modelRequest.current;
    if(catalog&&selection===connectionId){setModels(catalog);setLoadingModels(false);return;}
    setModels({data:[]});if(!desktop)return;setLoadingModels(true);
    const args={agent,connectionId:selection,sessionId:current.current?.id??null};
    void (async()=>{
      const source=await call<ModelSource>('get_model_source',args);
      if(disposed||revision!==modelRequest.current)return;
      setModels({data:[],source});if(source?.available!==true)return;
      const result=await call<ModelCatalog>('list_models',args);
      if(!disposed&&revision===modelRequest.current)setModels(result);
    })().catch(e=>{if(!disposed&&revision===modelRequest.current)report(e);}).finally(()=>{if(!disposed&&revision===modelRequest.current)setLoadingModels(false);});
    return()=>{disposed=true;};
  },[agent,selection,connectionId,catalog]);
  async function reloadModels(force=false){
    const revision=++modelRequest.current;setLoadingModels(true);
    try{const result=await call<ModelCatalog>('list_models',{agent,connectionId:selection,sessionId:current.current?.id??null,force});if(revision===modelRequest.current)setModels(result);}
    catch(e){if(revision===modelRequest.current)report(e);}finally{if(revision===modelRequest.current)setLoadingModels(false);}
  }
  async function chooseModel(option:ModelOption){
    if(busy||configuringLock.current)return;setConfiguring(true);
    try{if(current.current){const saved=await call<Session>('switch_session_model',{sessionId:current.current.id,connectionId:option.connectionId,model:option.model});current.current=saved;setSession(saved);await refresh();}
      setSelection(option.connectionId);updateOptions({...options.current,model:option.model,effort:option.defaultReasoningEffort??null});}
    catch(e){report(e);}finally{setConfiguring(false);}
  }
  function addAttachments(items:(Attachment|File)[]){
    if(!desktop)return Promise.reject(new Error('请在桌面版中上传或粘贴图片'));
    pendingImports.current++;setImporting(true);
    const job=attachmentJobs.current.then(async()=>{
      if(items.length+options.current.attachments.length>MAX_ATTACHMENTS)throw new Error('最多添加 12 个附件，请先移除一个附件');
      for(const item of items)if(item instanceof File)validateImageFile(item);
      const imported:Attachment[]=[];
      for(const item of items){
        if(!(item instanceof File)&&item.kind!=='image'){imported.push(item);continue;}
        const remainingBytes=MAX_TOTAL_IMAGE_BYTES-imageBytes(mergeAttachments(options.current.attachments,imported));
        const args=item instanceof File?{data:await imageBase64(item),name:item.name,remainingBytes}:{path:item.path,name:item.name,remainingBytes};
        imported.push(await call<Attachment>('import_image_attachment',args));
      }
      updateOptions({...options.current,attachments:mergeAttachments(options.current.attachments,imported)});
    }).finally(()=>{pendingImports.current--;setImporting(pendingImports.current>0);});
    attachmentJobs.current=job.catch(()=>{});return job;
  }
  async function startGoal(objective:string){
    if(busy||configuringLock.current||sendLock.current)return;sendLock.current=true;setSending(true);setError('');
    let s=current.current;const route=connectionRoute(s),sent=options.current,operation=beginPendingConnection(route);
    try{await requireChatConnection(route.agent,route.id,s?.id);if(!operation.active())return;
      if(!s){s=await createChatSession({projectId,agent:route.agent,connectionId:route.id,model:sent.model??''});if(!operation.active())return;current.current=s;setSession(s);}
      await call('execute_agent_command',{sessionId:s.id,command:'goal-create',argument:objective,permissionMode:sent.permissionMode});if(operation.active())await refresh();}
    catch(e){if(operation.active())report(e);}finally{const active=operation.active();operation.finish();sendLock.current=false;if(active)setSending(false);}
  }
  function report(e:unknown){if(isProviderRequired(e)){providerRequired(agent,savedDraft());return;}setError(e instanceof Error?e.message:String(e));}
  const questionPending=requests.some(r=>r.method==='item/tool/requestUserInput');
  async function respondRequest(request:RpcEvent,result:unknown){
    const execute=request.method==='claude/tool/requestApproval'&&request.params.toolName==='ExitPlanMode'&&(result as {decision?:string})?.decision==='accept';
    const mode=planExecutionPermission(executionPermission.current,agent);
    const reply=execute?{...(result as Record<string,unknown>),executionPermission:mode}:result;
    await call('respond_request',{id:request.id,result:reply});if(execute)permission(mode);resolveRequest(request.id!);await refresh();
  }
  const followups=useFollowups(session?.id??'',report);
  const followupEditor=useFollowupEditor(session?.id??'',{
    getDraft:()=>({...options.current,text:inputRef.current}),
    setDraft:next=>{updateOptions(next);setInput(next.text);},
    reload:followups.reload,report,focus:()=>textarea.current?.focus(),
  });
  const stopMode=isActive(session?.status)&&!!session?.turnId&&!hasDraft&&!followupEditor.editingId;
  useEffect(()=>{if(area.current&&stickToBottom.current)area.current.scrollTop=area.current.scrollHeight;},[messages]);
  const respondPlan = useStableCallback(async (message:Message,decision:PlanDecision,feedback:string) => {
    const s=current.current;
    if(!s || agent!=='codex' || configuringLock.current || busy || sendLock.current || followups.rows.length || message.sessionId!==s.id)throw new Error('请等待当前任务结束后再确认计划。');
    const mode=decision==='execute'?planExecutionPermission(executionPermission.current,agent):'read';
    const sent={...options.current,text:planFollowup(decision,feedback),attachments:[],permissionMode:mode,readOnly:mode==='read'};
    sendLock.current=true;setSending(true);
    const route=connectionRoute(s),operation=beginPendingConnection(route);
    try {await requireChatConnection(route.agent,route.id,s.id);if(!operation.active())return;await call('enqueue_followup',{sessionId:s.id,payload:sent});if(!operation.active())return;permission(mode);await followups.reload();await refresh();}
    finally {const active=operation.active();operation.finish();sendLock.current=false;if(active)setSending(false);}
  });
  async function change(id:string,action:string,text?:string){const operation=action==='resume'?beginPendingConnection():undefined;try{await call('change_followup',{id,action,text:text??null});await followups.reload();}catch(e){report(e);throw e;}finally{operation?.finish();}}
  async function stop(){const s=current.current;if(!s||stopping)return;setStopping(true);try{await call('interrupt_turn',{sessionId:s.id});}catch(e){setError(String(e));}finally{setStopping(false);}}
  async function send(){
    if(configuringLock.current||sendLock.current||importing||!hasDraft||followupEditor.busy)return;
    const sent={...options.current,text:inputRef.current};if(followupEditor.editingId){await withPendingConnection(()=>followupEditor.save(sent));return;}
    sendLock.current=true;setSending(true);setError('');let s=current.current;const route=connectionRoute(s),operation=beginPendingConnection(route);
    try{await requireChatConnection(route.agent,route.id,s?.id);if(!operation.active())return;
      if(!s){s=await createChatSession({projectId,agent:route.agent,connectionId:route.id,model:sent.model??''});if(!operation.active())return;current.current=s;setSession(s);}
      await call('enqueue_followup',{sessionId:s.id,payload:sent});if(!operation.active())return;
      if(source.current){await call('change_followup',{id:source.current,action:'remove',text:null});source.current=undefined;}
      updateOptions({...options.current,attachments:options.current.attachments.filter(a=>!sent.attachments.some(s=>s.path===a.path))});if(inputRef.current===sent.text)setInput('');await followups.reload();await refresh();
    }catch(e){if(operation.active())report(e);}finally{const active=operation.active();operation.finish();sendLock.current=false;if(active)setSending(false);}
  }
  return <PlanDisplayContext.Provider value={planDisplay}><aside className="side-chat" aria-label="侧边聊天" hidden={hidden} inert={hidden}>{resizeHandle}
    <header className="side-chat-header"><MessageSquare size={16}/><strong>侧边聊天</strong>
      {!embedded ? <button className="icon-button" aria-label="关闭侧边聊天" title="关闭侧边聊天" onClick={()=>{if(session)opened(session);close();}}><X size={16}/></button> : null}
    </header>
    <div className="side-chat-scroll" ref={area} onClickCapture={e=>{if((e.target as Element).closest('.turn-process-toggle, .activity-group-summary, .activity-row, .activity-toggle-more'))stickToBottom.current=false;}} onScroll={()=>{const el=area.current;if(el)stickToBottom.current=el.scrollHeight-el.scrollTop-el.clientHeight<100;}}>
      {messages.length||busy?<Conversation messages={messages} agentName={agentName} autoExpand={false} projectId={projectId} sessionId={session?.id} sessionStatus={sending&&!isActive(session?.status)?'starting':session?.status} pendingLabel={sending&&!isActive(session?.status)?'正在发送…':undefined} activeTurnId={session?.turnId} openFile={openSessionFile} showDiff={showDiff} onPlanAction={agent==='codex' && !busy && !configuring && !followups.rows.length ? respondPlan : undefined}/>
        :<div className="side-chat-welcome"><MessageSquare size={24} strokeWidth={1.5}/><h2>继续探讨</h2><p>围绕选中内容开启一个独立对话</p></div>}
      {requests.filter(r=>r.method!=='item/tool/requestUserInput').map(r=><Suspense key={String(r.id)} fallback={<p>正在加载确认请求…</p>}><RequestCard request={r} respond={result=>respondRequest(r,result)} openFile={openSessionFile} messages={messages}/></Suspense>)}
    </div>
    <footer className={'composer-area side-chat-composer-area'+(questionPending?' has-question':'')}>
      {error?<div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="关闭侧边聊天提示" onClick={()=>setError('')}><X size={14}/></button></div>:null}
      {followups.rows.length&&!session?.turnId?<p className="side-chat-queue-note">已排队，将在当前任务完成后发送</p>:null}
      {questionPending?<Suspense fallback={<p>正在加载提问…</p>}><QuestionDock requests={requests} respond={respondRequest}/></Suspense>:null}
      <div className="plan-decision-dock" ref={setPlanDock}/>
      <FollowupQueue rows={followups.rows} agent={agent} turnId={session?.turnId} steeringMode={followups.steeringMode} change={change} steer={async id=>{try{await call('steer_followup',{id,expectedTurnId:session?.turnId});await followups.reload();}catch(e){report(e);}}} openSide={row=>void followupEditor.begin(row)} edit={followupEditor.begin} editingId={followupEditor.editingId}/>
      <form className={'composer side-chat-composer'+(busy?' busy':'')} onSubmit={e=>{e.preventDefault();void send();}}>
        {followupEditor.editingId?<div className="composer-editing"><span>编辑排队消息</span><button type="button" disabled={followupEditor.busy} onClick={()=>void withPendingConnection(()=>followupEditor.cancel())}>取消编辑</button></div>:null}
        <AttachmentStrip attachments={payload.attachments} loading={importing} remove={removeAttachment}/>
        <div className="composer-input"><ComposerSkills attachments={payload.attachments} remove={removeAttachment}/>
          <ComposerTextarea ref={textarea} disabled={followupEditor.busy} autoFocus aria-label="侧边聊天消息" value={input} rows={questionPending?1:2} placeholder={questionPending?'随心输入':payload.permissionMode==='read'&&(agent==='claude'||agent==='codex')?'描述你的任务以生成计划…':'继续提问…'} onChange={e=>setInput(e.target.value)} onPaste={e=>{const images=clipboardImages(e.clipboardData);if(images.length){e.preventDefault();void addAttachments(images).catch(report);}}} onKeyDown={e=>{if(e.key==='Enter'&&(enterSend||e.ctrlKey||e.metaKey)&&!e.shiftKey&&!e.nativeEvent.isComposing){e.preventDefault();void send();}}}/>
        </div>
        <div className="composer-toolbar"><div className="composer-options">
          <AddMenu projectId={projectId} agent={agent} permissionMode={payload.permissionMode as PermissionMode} attachments={payload.attachments} busy={sending||importing} modeBusy={busy||configuring} addAttachments={addAttachments} onSkill={skill=>{if(skill.error){report(skill.error);return false;}try{updateOptions({...options.current,attachments:mergeAttachments(options.current.attachments,[{kind:'skill',name:skill.name,path:skill.path}])});textarea.current?.focus();return true;}catch(e){report(e);return false;}}} onPlan={enabled=>permission(enabled?'read':executionPermission.current)} onGoal={objective=>void startGoal(objective)}/>
          <PermissionMenu value={payload.permissionMode as PermissionMode} agent={agent} busy={busy||configuring} onChange={permission} label="侧边聊天权限模式"/>
        </div><div className="composer-right">
          <span className="side-chat-agent" title={agentName}><AgentIcon agent={agent}/><span>{agentName}</span></span>
          <ModelMenu value={payload.model??''} models={models.data} catalogLoaded={models.origin!=null} catalogWarning={models.warning||models.notice} profiles={profiles} activeProfile={profiles.find(p=>p.id===selection&&p.agent===agent)} connectionId={selection} source={models.source} agent={agent} busy={busy||configuring} loading={loadingModels} choose={chooseModel} reload={force=>void reloadModels(force)} manage={()=>(manageProviders??providerRequired)(agent,savedDraft())} effort={payload.effort??''} setEffort={effort=>updateOptions({...options.current,effort:effort||null})} hasConversation={!!session?.nativeId}/>
          <button className={'send-button'+(stopMode?' stop-button':'')} type={stopMode?'button':'submit'} disabled={stopMode?!session?.turnId||stopping:sending||importing||configuring||followupEditor.busy||!hasDraft} onClick={stopMode?()=>void stop():undefined} aria-label={stopMode?'停止侧边聊天任务':followupEditor.editingId?'发送侧边聊天修改':'发送侧边聊天消息'} title={stopMode?'停止生成':isActive(session?.status)&&hasDraft?'加入队列，当前任务完成后发送':'发送消息'}>{stopMode?<Square size={14} fill="currentColor"/>:<ArrowUp size={18}/>}</button>
        </div></div>
      </form>
    </footer>
  </aside></PlanDisplayContext.Provider>;
}
