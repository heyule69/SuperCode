import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowUp, ExternalLink, MessageSquare, Square, X } from 'lucide-react';
import { call, desktop, subscribe } from './api';
import { ComposerTextarea } from './ComposerTextarea';
import { AddMenu, AttachmentStrip, ComposerSkills, ModelMenu, PermissionMenu } from './ComposerMenus';
import { AgentIcon } from './AgentIcon';
import { clipboardImages, imageBase64, imageBytes, MAX_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES, mergeAttachments, validateImageFile, type Attachment } from './attachments';
import type { ModelOption } from './modelPicker';
import type { AgentProfile, ModelCatalog, ModelSource, PermissionMode } from './types';

const emptyProfiles: AgentProfile[] = [];
const agentNames: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', opencode: 'OpenCode', pi: 'Pi' };
import { createChatSession } from './chatSession';
import { isProviderRequired, requireChatConnection } from './chatConnection';
import { planFollowup, type PlanDecision } from './planPresentation';
import { Conversation } from './Conversation';
import { mergeEvent } from './events';
import { FollowupQueue, useFollowups, type FollowupPayload } from './Followups';
import { useFollowupEditor } from './useFollowupEditor';
import { isActive, type Bootstrap, type Message, type RuntimeInfo, type Session, type RpcEvent } from './types';
const RequestCard=lazy(()=>import('./RequestCard'));
const QuestionDock=lazy(()=>import('./QuestionDock'));
export interface SideDraft {key:string;text:string;payload?:FollowupPayload;sourceId?:string;projectId?:string;agent?:string;connectionId?:string}
export default function SideChat({draft,projectId,agent,connectionId,defaults,close,opened,openFile,showDiff,resizeHandle,providerRequired,manageProviders,profiles=emptyProfiles,catalog,enterSend=true}:{draft:SideDraft;projectId:string;agent:string;connectionId:string;defaults:FollowupPayload;close:()=>void;opened:(session:Session)=>void;openFile:(path:string,line?:number,workspaceSessionId?:string)=>void;resizeHandle?:ReactNode;showDiff:(name:string,text:string)=>void;providerRequired:(agent:string,draft:SideDraft)=>void;manageProviders?:(agent:string,draft:SideDraft)=>void;profiles?:AgentProfile[];catalog?:ModelCatalog;enterSend?:boolean}){
  const [input,setInput]=useState(draft.text);const [session,setSession]=useState<Session>();const [messages,setMessages]=useState<Message[]>([]);const [requests,setRequests]=useState<RpcEvent[]>([]);const [error,setError]=useState('');const [sending,setSending]=useState(false);const [stopping,setStopping]=useState(false);
  const current=useRef(session);current.current=session;const area=useRef<HTMLDivElement>(null);const stickToBottom=useRef(true);const timer=useRef<ReturnType<typeof setTimeout>|undefined>(undefined);
  const [payload,setPayload]=useState(draft.payload??defaults);
  const options=useRef(payload);options.current=payload;const source=useRef(draft.sourceId);
  const inputRef=useRef(input);inputRef.current=input;
  const [selection,setSelection]=useState(connectionId);const [models,setModels]=useState<ModelCatalog>(catalog??{data:[]});
  const [loadingModels,setLoadingModels]=useState(false);const [configuring,setConfiguring]=useState(false);const [importing,setImporting]=useState(false);
  const textarea=useRef<HTMLTextAreaElement>(null);const modelRequest=useRef(0);const sendLock=useRef(false);
  const attachmentJobs=useRef(Promise.resolve());const pendingImports=useRef(0);
  const hasDraft=!!input.trim()||payload.attachments.length>0;const busy=sending||isActive(session?.status);
  const agentName=agentNames[agent]??agent;
  function savedDraft():SideDraft{return {...draft,text:inputRef.current,connectionId:selection,payload:options.current,sourceId:source.current};}
  function updateOptions(next:FollowupPayload){options.current=next;setPayload(next);}
  function permission(mode:PermissionMode){updateOptions({...options.current,permissionMode:mode,readOnly:mode==='read'});}
  function removeAttachment(path:string){updateOptions({...options.current,attachments:options.current.attachments.filter(a=>a.path!==path)});textarea.current?.focus();}
  useEffect(()=>{if(!current.current)setSelection(connectionId);},[connectionId]);
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
    if(busy||configuring)return;setConfiguring(true);
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
    if(busy||sendLock.current)return;sendLock.current=true;setSending(true);setError('');
    try{await requireChatConnection(agent,selection,current.current?.id);let s=current.current;
      if(!s){s=await createChatSession({projectId,agent,connectionId:selection,model:options.current.model??''});current.current=s;setSession(s);}
      await call('execute_agent_command',{sessionId:s.id,command:'goal-create',argument:objective,permissionMode:options.current.permissionMode});await refresh();}
    catch(e){report(e);}finally{sendLock.current=false;setSending(false);}
  }
  function report(e:unknown){if(isProviderRequired(e)){providerRequired(agent,savedDraft());return;}setError(e instanceof Error?e.message:String(e));}
  const questionPending=requests.some(r=>r.method==='item/tool/requestUserInput');
  async function respondRequest(request:RpcEvent,result:unknown){await call('respond_request',{id:request.id,result});setRequests(old=>old.filter(r=>r.id!==request.id));await refresh();}
  const followups=useFollowups(session?.id??'',report);
  const followupEditor=useFollowupEditor(session?.id??'',{
    getDraft:()=>({...options.current,text:inputRef.current}),
    setDraft:next=>{updateOptions(next);setInput(next.text);},
    reload:followups.reload,report,focus:()=>textarea.current?.focus(),
  });
  const stopMode=isActive(session?.status)&&!!session?.turnId&&!hasDraft&&!followupEditor.editingId;
  async function refresh(){const s=current.current;if(!s)return;const [data,items,info]=await Promise.all([call<Bootstrap>('bootstrap'),call<Message[]>('list_messages',{sessionId:s.id,limit:120}),call<RuntimeInfo>('runtime_info')]);const saved=data.sessions.find(x=>x.id===s.id);if(saved){current.current=saved;setSession(saved);setRequests(info.requests.filter(r=>r.params?.threadId===saved.nativeId));}setMessages(items);}
  useEffect(()=>{let disposed=false;let off:(()=>void)|undefined;const schedule=()=>{if(!current.current||timer.current)return;timer.current=setTimeout(()=>{timer.current=undefined;void refresh().catch(e=>setError(String(e)));},150);};void subscribe(e=>{if(e.method==='turn/started'||e.method==='turn/completed'){schedule();return;}const s=current.current;if(s&&e.params?.threadId===s.nativeId){setMessages(old=>mergeEvent(old,e,s.id));if(e.id!=null)setRequests(old=>old.some(r=>r.id===e.id)?old:[...old,e]);if(e.method==='serverRequest/resolved')setRequests(old=>old.filter(r=>r.id!==e.params?.requestId));}},()=>{},schedule,schedule).then(fn=>{if(disposed)fn();else off=fn;});return()=>{disposed=true;off?.();if(timer.current)clearTimeout(timer.current);};},[]);
  useEffect(()=>{if(area.current&&stickToBottom.current)area.current.scrollTop=area.current.scrollHeight;},[messages]);
  async function respondPlan(message:Message,decision:PlanDecision,feedback:string) {
    const s=current.current;
    if(!s || agent!=='codex' || busy || sendLock.current || followups.rows.length || message.sessionId!==s.id)throw new Error('请等待当前任务结束后再确认计划。');
    const mode=decision==='execute'?'ask':'read';
    const sent={...options.current,text:planFollowup(decision,feedback),attachments:[],permissionMode:mode,readOnly:mode==='read'};
    sendLock.current=true;setSending(true);
    try {await requireChatConnection(agent,selection,s.id);await call('enqueue_followup',{sessionId:s.id,payload:sent});permission(mode);await followups.reload();await refresh();}
    finally {sendLock.current=false;setSending(false);}
  }
  async function change(id:string,action:string,text?:string){try{await call('change_followup',{id,action,text:text??null});await followups.reload();}catch(e){report(e);throw e;}}
  async function stop(){const s=current.current;if(!s||stopping)return;setStopping(true);try{await call('interrupt_turn',{sessionId:s.id});}catch(e){setError(String(e));}finally{setStopping(false);}}
  async function send(){if(sendLock.current||importing||!hasDraft||followupEditor.busy)return;const sent={...options.current,text:inputRef.current};if(followupEditor.editingId){await followupEditor.save(sent);return;}sendLock.current=true;setSending(true);setError('');try{let s=current.current;await requireChatConnection(agent,selection,s?.id);if(!s){s=await createChatSession({projectId,agent,connectionId:selection,model:options.current.model??''});current.current=s;setSession(s);}await call('enqueue_followup',{sessionId:s.id,payload:sent});if(source.current){await call('change_followup',{id:source.current,action:'remove',text:null});source.current=undefined;}updateOptions({...options.current,attachments:options.current.attachments.filter(a=>!sent.attachments.some(s=>s.path===a.path))});if(inputRef.current===sent.text)setInput('');await followups.reload();await refresh();}catch(e){report(e);}finally{sendLock.current=false;setSending(false);}}
  return <aside className="side-chat" aria-label="侧边聊天">{resizeHandle}
    <header className="side-chat-header"><MessageSquare size={16}/><strong>侧边聊天</strong>
      {session?<button className="icon-button" aria-label="打开侧边聊天的独立窗口" title="在独立窗口中打开" onClick={()=>void call('open_chat_window',{sessionId:session.id}).catch(report)}><ExternalLink size={16}/></button>:null}
      <button className="icon-button" aria-label="收起侧边聊天" title="收起侧边聊天" onClick={()=>{if(session)opened(session);close();}}><X size={16}/></button>
    </header>
    <div className="side-chat-scroll" ref={area} onClickCapture={e=>{if((e.target as Element).closest('.turn-process-toggle, .activity-group-summary, .activity-row, .activity-toggle-more'))stickToBottom.current=false;}} onScroll={()=>{const el=area.current;if(el)stickToBottom.current=el.scrollHeight-el.scrollTop-el.clientHeight<100;}}>
      {messages.length||busy?<Conversation messages={messages} agentName={agentName} autoExpand={false} projectId={projectId} sessionId={session?.id} sessionStatus={sending&&!isActive(session?.status)?'starting':session?.status} pendingLabel={sending&&!isActive(session?.status)?'正在发送…':undefined} activeTurnId={session?.turnId} openFile={(path,line)=>openFile(path,line,session?.id)} showDiff={showDiff} onPlanAction={agent==='codex' && !busy && !followups.rows.length ? respondPlan : undefined}/>
        :<div className="side-chat-welcome"><MessageSquare size={24} strokeWidth={1.5}/><h2>继续探讨</h2><p>围绕选中内容开启一个独立对话</p></div>}
      {requests.filter(r=>r.method!=='item/tool/requestUserInput').map(r=><Suspense key={String(r.id)} fallback={<p>正在加载确认请求…</p>}><RequestCard request={r} respond={result=>respondRequest(r,result)} openFile={(path,line)=>openFile(path,line,session?.id)} messages={messages}/></Suspense>)}
    </div>
    <footer className={'composer-area side-chat-composer-area'+(questionPending?' has-question':'')}>
      {error?<div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="关闭侧边聊天提示" onClick={()=>setError('')}><X size={14}/></button></div>:null}
      {followups.rows.length&&!session?.turnId?<p className="side-chat-queue-note">已排队，将在当前任务完成后发送</p>:null}
      {questionPending?<Suspense fallback={<p>正在加载提问…</p>}><QuestionDock requests={requests} respond={respondRequest}/></Suspense>:null}
      <FollowupQueue rows={followups.rows} agent={agent} turnId={session?.turnId} steeringMode={followups.steeringMode} change={change} steer={async id=>{try{await call('steer_followup',{id,expectedTurnId:session?.turnId});await followups.reload();}catch(e){report(e);}}} openSide={row=>void followupEditor.begin(row)} edit={followupEditor.begin} editingId={followupEditor.editingId}/>
      <form className={'composer side-chat-composer'+(busy?' busy':'')} onSubmit={e=>{e.preventDefault();void send();}}>
        {followupEditor.editingId?<div className="composer-editing"><span>编辑排队消息</span><button type="button" disabled={followupEditor.busy} onClick={()=>void followupEditor.cancel()}>取消编辑</button></div>:null}
        <AttachmentStrip attachments={payload.attachments} loading={importing} remove={removeAttachment}/>
        <div className="composer-input"><ComposerSkills attachments={payload.attachments} remove={removeAttachment}/>
          <ComposerTextarea ref={textarea} disabled={followupEditor.busy} autoFocus aria-label="侧边聊天消息" value={input} rows={questionPending?1:2} placeholder={questionPending?'随心输入':'继续提问…'} onChange={e=>setInput(e.target.value)} onPaste={e=>{const images=clipboardImages(e.clipboardData);if(images.length){e.preventDefault();void addAttachments(images).catch(report);}}} onKeyDown={e=>{if(e.key==='Enter'&&(enterSend||e.ctrlKey||e.metaKey)&&!e.shiftKey&&!e.nativeEvent.isComposing){e.preventDefault();void send();}}}/>
        </div>
        <div className="composer-toolbar"><div className="composer-options">
          <AddMenu projectId={projectId} agent={agent} permissionMode={payload.permissionMode as PermissionMode} attachments={payload.attachments} busy={sending||importing} modeBusy={busy||configuring} addAttachments={addAttachments} onSkill={skill=>{if(skill.error){report(skill.error);return false;}try{updateOptions({...options.current,attachments:mergeAttachments(options.current.attachments,[{kind:'skill',name:skill.name,path:skill.path}])});textarea.current?.focus();return true;}catch(e){report(e);return false;}}} onPlan={enabled=>permission(enabled?'read':'ask')} onGoal={objective=>void startGoal(objective)}/>
          <PermissionMenu value={payload.permissionMode as PermissionMode} agent={agent} busy={busy||configuring} onChange={permission} label="侧边聊天权限模式"/>
        </div><div className="composer-right">
          <span className="side-chat-agent" title={agentName}><AgentIcon agent={agent}/><span>{agentName}</span></span>
          <ModelMenu value={payload.model??''} models={models.data} catalogLoaded={models.origin!=null} catalogWarning={models.warning||models.notice} profiles={profiles} activeProfile={profiles.find(p=>p.id===selection&&p.agent===agent)} connectionId={selection} source={models.source} agent={agent} busy={busy||configuring} loading={loadingModels} choose={chooseModel} reload={force=>void reloadModels(force)} manage={()=>(manageProviders??providerRequired)(agent,savedDraft())} effort={payload.effort??''} setEffort={effort=>updateOptions({...options.current,effort:effort||null})} hasConversation={!!session?.nativeId}/>
          <button className={'send-button'+(stopMode?' stop-button':'')} type={stopMode?'button':'submit'} disabled={stopMode?!session?.turnId||stopping:sending||importing||configuring||followupEditor.busy||!hasDraft} onClick={stopMode?()=>void stop():undefined} aria-label={stopMode?'停止侧边聊天任务':followupEditor.editingId?'发送侧边聊天修改':'发送侧边聊天消息'} title={stopMode?'停止生成':isActive(session?.status)&&hasDraft?'加入队列，当前任务完成后发送':'发送消息'}>{stopMode?<Square size={14} fill="currentColor"/>:<ArrowUp size={18}/>}</button>
        </div></div>
      </form>
    </footer>
  </aside>;
}
