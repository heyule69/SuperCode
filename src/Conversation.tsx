import { lazy, memo, Suspense, useEffect, useMemo, useState } from 'react';
import { ArrowDown, Brain, Check, ChevronDown, ChevronRight, Circle, FileCode2, FileDiff, FileSearch, Globe, ListChecks, MessageCircleQuestion, Monitor, Network, Pencil, Shield, Square, SquareTerminal, Wrench, X } from 'lucide-react';
import { conversationTurns, isActivity, type ConversationTurn } from './events';
import { isActive, type Message } from './types';
import { diffFiles } from './diff';
import { CopyButton } from './CopyButton';
import { activityStatus, compactActivities, concisePath, summarizeActivities } from './activityPresentation';
import { ResourceMenu, type OpenPath } from './ResourceMenu';
import { ConversationIndex } from './ConversationIndex';
import { AttachmentImage } from './AttachmentImage';
import { MediaContext, MediaList } from './Media';
import { messageMedia } from './mediaSources';
import { presentTurn, type TurnLifecycle } from './turnPresentation';
import { QuestionHistory } from './QuestionHistory';
import { planText, type PlanDecision } from './planPresentation';
import { collectSubagents, isSubagent } from './subagents';
import { SubagentContext, SubagentRows } from './SubagentRow';
const PlanDocument = lazy(() => import('./PlanDocument').then(module => ({ default: module.PlanDocument })));
const ProposedPlan = lazy(() => import('./ProposedPlan'));

const Markdown = lazy(() => import('./Markdown'));
const pending = (status: unknown) => ['inProgress', 'running', 'preparing'].includes(String(status));
const stringify = (value: unknown) => typeof value === 'string' ? value : value == null ? '' : JSON.stringify(value, null, 2).slice(0, 128 * 1024);
const duration = (ms: number) => { const seconds = Math.max(1, Math.round(ms / 1000)); return seconds < 60 ? `${seconds} 秒` : `${Math.floor(seconds / 60)} 分钟 ${seconds % 60} 秒`; };
type ProcessDisplay = 'collapsible' | 'timeline';

export function activityTitle(m: Message) {
  const d = m.data ?? {};
  const name = String(d.tool ?? '');
  if (m.kind === 'reasoning') return { label: '思考摘要', detail: '', icon: Brain };
  if (m.kind === 'executionPlan') return { label: '执行计划', detail: '', icon: ListChecks };
  if (m.kind === 'claudeToolCall') {
    const args = (d.arguments ?? {}) as Record<string, unknown>;
    const labels: Record<string, string> = { Read: '读取文件', Glob: '查找文件', Grep: '搜索代码', Edit: '编辑文件', Write: '写入文件', Bash: '执行命令', AskUserQuestion: '询问用户', EnterPlanMode: '开始规划', ExitPlanMode: '提交计划' };
    return { label: labels[name] ?? (name || '调用工具'), detail: String(args.file_path ?? args.command ?? args.pattern ?? args.path ?? ''), icon: name === 'AskUserQuestion' ? MessageCircleQuestion : ['EnterPlanMode', 'ExitPlanMode'].includes(name) ? ListChecks : name === 'Bash' ? SquareTerminal : ['Read', 'Glob', 'Grep'].includes(name) ? FileSearch : ['Edit', 'Write'].includes(name) ? FileCode2 : Wrench };
  }
  if (m.kind === 'commandExecution') {
    const actions = d.commandActions as { type: string; path?: string; query?: string }[] | undefined;
    const action = actions?.[0];
    const label = action?.type === 'read' ? '读取文件' : action?.type === 'search' ? '搜索代码' : action?.type === 'listFiles' ? '浏览文件' : '执行命令';
    return { label, detail: String(action?.path ?? action?.query ?? d.command ?? m.text.split('\n')[0]), icon: action && action.type !== 'unknown' ? FileSearch : SquareTerminal };
  }
  if (m.kind === 'fileChange') return { label: '修改文件', detail: m.text.split('\n')[0], icon: FileCode2 };
  if (m.kind === 'webSearch') return { label: '搜索网页', detail: m.text || stringify(d.action), icon: Globe };
  if (m.kind === 'imageView') return { label: '查看图片', detail: m.text, icon: FileSearch };
  if (m.kind.startsWith('collab')) return { label: '协作 Agent', detail: name, icon: Network };
  if (m.kind === 'contextCompaction') return { label: '整理上下文', detail: '', icon: ListChecks };
  if (m.kind.includes('ReviewMode')) return { label: m.kind === 'enteredReviewMode' ? '开始代码审查' : '结束代码审查', detail: m.text, icon: Shield };
  return { label: name || '调用工具', detail: String(d.server ?? d.namespace ?? ''), icon: Wrench };
}

function StatusIcon({ status }: { status: string }) {
  return pending(status) ? null : ['failed', 'declined'].includes(status) ? <X size={14} /> : status === 'interrupted' ? <Square size={12} /> : <Check size={14} />;
}

const ActivityItem = memo(function ActivityItem({ message: m, autoExpand }: { message: Message; autoExpand: boolean }) {
  const [expanded, setExpanded] = useState(autoExpand || m.kind === 'claudeToolCall' && m.data?.tool === 'ExitPlanMode' && m.data.status === 'preparing');
  useEffect(() => setExpanded(autoExpand || m.kind === 'claudeToolCall' && m.data?.tool === 'ExitPlanMode' && m.data.status === 'preparing'), [autoExpand, m.id]);

  const d = m.data ?? {};
  const status = activityStatus(m);
  const { label, detail, icon: Icon } = activityTitle(m);
  const reasoning = m.kind === 'reasoning';
  const plan = m.kind === 'executionPlan';
  const question = m.kind === 'claudeToolCall' && d.tool === 'AskUserQuestion';
  const planSubmission = m.kind === 'claudeToolCall' && d.tool === 'ExitPlanMode';
  const document = planSubmission && expanded ? planText(d.arguments) || planText(d.inputText, status === 'preparing') : '';
  const output = m.kind === 'commandExecution' ? m.text.slice(String(d.command ?? m.text.split('\n')[0]).length).replace(/^\n/, '') : m.kind === 'fileChange' ? String(d.output ?? '') : m.kind === 'claudeToolCall' ? m.text : stringify(d.result ?? d.contentItems ?? d.error);
  const args = question || planSubmission ? '' : d.inputText && status === 'preparing' ? String(d.inputText) : stringify(d.arguments);
  const changes = Array.isArray(d.changes) ? d.changes as { path: string; kind?: { type?: string } | string; diff?: string }[] : [];
  const steps = Array.isArray(d.plan) ? d.plan as { step: string; status: string }[] : [];
  const stateText = status === 'preparing' ? question ? '生成问题' : planSubmission ? '生成计划' : '生成参数' : pending(status) ? reasoning ? '思考中' : '运行中' : status === 'failed' ? '失败' : status === 'interrupted' ? '已停止' : status === 'declined' ? '已拒绝' : '完成';
  const finishedLabels: Record<string, string> = { '执行命令': '运行了命令', '读取文件': '读取了文件', '编辑文件': '编辑了文件', '写入文件': '写入了文件', '修改文件': '修改了文件', '查找文件': '查找了文件', '搜索代码': '搜索了代码', '搜索网页': '搜索了网页', '查看图片': '查看了图片' };
  const rowLabel = pending(status) || ['failed', 'declined', 'interrupted'].includes(status) ? label : finishedLabels[label] ?? label;
  const hasBody = question || planSubmission || reasoning || plan || !!(output || args || changes.length || d.progress || d.terminalInput || d.cwd || d.prompt || d.agentsStates || (m.kind === 'commandExecution' && detail));
  const copyText = question ? '' : planSubmission ? document : [detail, args, reasoning ? m.text : output, ...changes.map(c => `${c.path}\n${c.diff ?? ''}`)].filter(Boolean).join('\n\n');
  const preview = label === '执行命令' ? '' : reasoning ? m.text.replace(/\s+/g, ' ').trim().slice(0, 160) : m.kind === 'fileChange' ? changes.map(c => concisePath(c.path)).join('、') || concisePath(detail) : ['读取文件', '编辑文件', '写入文件', '查找文件', '查看图片'].includes(label) ? concisePath(detail) : detail;
  return <section className={`activity-item ${reasoning ? 'reasoning' : ''} ${status}`} aria-label={`${label}：${stateText}`}>
    <button className="activity-row" type="button" aria-label={`${label}${preview ? ` · ${preview.slice(0, 160)}` : ''} · ${stateText}`} aria-expanded={hasBody && expanded} onClick={() => setExpanded(v => !v)} disabled={!hasBody}>
      <Icon size={14} /><strong>{rowLabel}</strong>{preview ? <code className={reasoning ? 'activity-summary-detail' : undefined} title={reasoning ? undefined : detail.slice(0, 240)}>{preview}</code> : null}{pending(status) || ['failed', 'declined', 'interrupted'].includes(status) ? <span className="activity-state"><StatusIcon status={status} /><span className={pending(status) ? 'pending-text' : undefined}>{stateText}</span></span> : null}{hasBody ? <ChevronDown size={12} className={expanded ? 'expanded' : ''} /> : null}
    </button>
    {expanded && hasBody ? <div className="activity-body">
      {reasoning ? <div className="reasoning-text">{m.text || (d.redacted ? 'Agent 未公开此段思考内容。' : pending(status) ? '正在等待 Agent 返回思考摘要…' : 'Agent 未返回可展示的思考摘要。')}</div> : null}
      {plan ? <><p className="plan-explanation">{m.text}</p><ol className="plan-steps">{steps.map((step, i) => <li key={i} className={step.status}>{step.status === 'completed' ? <Check size={14} /> : <Circle size={12} />}<span className={step.status === 'inProgress' && pending(status) ? 'pending-text' : undefined}>{step.step}</span><small>{step.status === 'completed' ? '完成' : step.status === 'inProgress' ? pending(status) ? '进行中' : '未完成' : '待处理'}</small></li>)}</ol></> : null}
      {d.cwd ? <div className="activity-meta">工作目录 <code>{String(d.cwd)}</code></div> : null}
      {m.kind === 'commandExecution' && detail ? <div className="activity-section"><span>命令</span><pre>{detail}</pre></div> : null}
      {question ? <QuestionHistory message={m} /> : planSubmission ? <>
        {document ? <Suspense fallback={<p>正在加载计划文档…</p>}><PlanDocument text={document} streaming={status === 'preparing'} /></Suspense> : <div className="activity-placeholder">{status === 'preparing' ? '正在生成计划…' : 'Agent 未提供可展示的计划文档。'}</div>}
      </> : <>
        {args ? <div className="activity-section"><span>输入参数{status === 'preparing' ? ' · 正在生成' : ''}</span><pre>{args}</pre></div> : null}
        {output ? <div className="activity-section"><span>{m.kind === 'claudeToolCall' ? '工具结果' : '输出'}{pending(status) ? ' · 实时更新' : ''}</span><pre>{output}</pre></div> : null}
        {pending(status) && !reasoning && !plan && !output ? <div className="activity-placeholder"><span className="pending-text">{status === 'preparing' ? '正在生成调用参数…' : '等待工具返回结果…'}</span></div> : null}
      </>}
      {changes.map(c => <div className="activity-section file-operation" key={c.path}><span><FileCode2 size={13} />{c.path}<small>{typeof c.kind === 'string' ? c.kind : c.kind?.type ?? '修改'}</small></span>{c.diff ? <pre>{c.diff.split('\n').map((line, i) => <span key={i} className={line.startsWith('+') ? 'added-line' : line.startsWith('-') ? 'removed-line' : ''}>{line}{'\n'}</span>)}</pre> : null}</div>)}
      {d.progress ? <div className="activity-section"><span>进度</span><pre>{String(d.progress)}</pre></div> : null}
      {d.terminalInput ? <div className="activity-section"><span>终端输入</span><pre>{String(d.terminalInput)}</pre></div> : null}
      {d.prompt ? <div className="activity-section"><span>协作任务</span><pre>{String(d.prompt)}</pre></div> : null}
      {d.agentsStates ? <div className="activity-section"><span>Agent 状态</span><pre>{stringify(d.agentsStates)}</pre></div> : null}
      <div className="activity-bottom"><span>{d.exitCode != null ? `退出码 ${d.exitCode}` : ''}{d.durationMs != null ? ` · ${duration(Number(d.durationMs))}` : ''}</span>{copyText && !pending(status) ? <CopyButton className="text-button" text={copyText} label="复制详情" /> : null}</div>
    </div> : null}
  </section>;
});

function ActivityGroup({ messages, autoExpand, live, processDisplay }: { messages: Message[]; autoExpand: boolean; live: boolean; processDisplay: ProcessDisplay }) {
  const [collapsed, setCollapsed] = useState(!autoExpand);
  const [showAll, setShowAll] = useState(false);
  useEffect(() => setCollapsed(!autoExpand), [autoExpand]);
  const { label, category, status } = summarizeActivities(messages, live);
  const Icon = { command: SquareTerminal, edit: Pencil, read: FileSearch, search: FileSearch, web: Globe, browser: Globe, computer: Monitor, plan: ListChecks, reasoning: Brain, context: ListChecks, tool: Wrench }[category];
  const stateText = status === 'failed' ? '调用失败' : status === 'declined' ? '已拒绝' : status === 'interrupted' ? '已停止' : '';
  const visible = autoExpand || showAll ? messages : compactActivities(messages);
  const media = [...new Map(messages.flatMap(messageMedia).map(ref => [ref.path, ref])).values()].slice(0, 12);
  const mediaErrors = messages.flatMap(m => Array.isArray(m.data?.mediaErrors) ? m.data.mediaErrors.filter((error: unknown) => typeof error === 'string') as string[] : []).slice(0, 3);
  const generatingPlan = live && collapsed && processDisplay === 'collapsible' ? [...messages].reverse().find(m => m.kind === 'claudeToolCall' && m.data?.tool === 'ExitPlanMode' && pending(m.data.status)) : undefined;
  const draftPlan = generatingPlan ? planText(generatingPlan.data?.arguments) || planText(generatingPlan.data?.inputText, true) : '';
  if (processDisplay === 'timeline') return <div className="activity-timeline">
    {messages.map(m => <ActivityItem key={m.id} message={m} autoExpand={m.kind === 'executionPlan'} />)}
    <MediaList media={media}/>{mediaErrors.map((error, index) => <p className="inline-error" role="status" key={index}>{error}</p>)}
  </div>;
  return <div className={`activity-group ${status}`}>
    <button className="activity-group-summary" aria-label={`${label}${stateText ? ` · ${stateText}` : ''}，${messages.length} 条记录`} aria-expanded={!collapsed} onClick={() => setCollapsed(value => !value)}>
      {category === 'reasoning' && pending(status) ? null : <Icon size={15}/>}<span className={pending(status) ? 'pending-text' : undefined}>{label}</span>{stateText ? <small>{stateText}</small> : null}<ChevronRight size={13} className={collapsed ? '' : 'expanded'}/>
    </button>
    {collapsed ? null : <div className="activity-items">{visible.map(m => <ActivityItem key={m.id} message={m} autoExpand={autoExpand} />)}{!autoExpand && messages.length > 3 ? <button className="activity-toggle-more" aria-expanded={showAll} onClick={() => setShowAll(value => !value)}>{showAll ? '收起' : `另 ${messages.length - visible.length} 项`}<ChevronDown size={11} className={showAll ? 'expanded' : ''} /></button> : null}</div>}
    {draftPlan ? <div className="plan-generation" aria-label="正在生成计划"><Suspense fallback={<span className="pending-text">正在加载计划文档…</span>}><PlanDocument text={draftPlan} streaming={generatingPlan?.data?.status === 'preparing'} /></Suspense></div> : null}
    <MediaList media={media}/>{mediaErrors.map((error, index) => <p className="inline-error" role="status" key={index}>{error}</p>)}
  </div>;
}

const MessageView = memo(function MessageView({ message: m, openFile }: { message: Message; openFile: (path: string, line?: number) => void }) {

  if (m.kind === 'modelSwitch') {
    const from = m.data?.from as { providerName?: string } | undefined;
    const to = m.data?.to as { providerName?: string; mark?: string } | undefined;
    const changed = from?.providerName !== to?.providerName;
    const compactOnly = m.data?.compactOnly === true;
    const context = ['native', 'history'].includes(String(m.data?.contextMode)) ? '上下文已保留' : '上下文已压缩';
    return <div className="model-switch-marker" role="note" title={`${context}，原始对话已保留。${String(m.data?.fromModel ?? '')} → ${String(m.data?.toModel ?? '')}`}><Check size={13}/><span>{context}</span>{compactOnly ? null : <span className="switch-marker-separator">·</span>}{!compactOnly && changed && from?.providerName ? <><span>{from.providerName}</span><span>→</span></> : null}{compactOnly ? null : <span>{changed ? to?.providerName : String(m.data?.toModel ?? '')}</span>}</div>;
  }
  if (m.kind === 'goalState' && !m.data?.goal) return null;
  const informational = m.kind === 'goalState' || m.kind === 'commandResult';
  const media = messageMedia(m);
  if (!m.text && m.role === 'assistant' && !media.length) return null;
  const attachments = Array.isArray(m.data?.attachments) ? m.data.attachments as {name:string;kind:string;path?:string}[] : [];
  const user = m.role === 'user';
  const images = user ? attachments.filter(a => a.kind === 'image' && a.path) : [];
  const inlineAttachments = user ? attachments.filter(a => a.kind !== 'image' || !a.path) : attachments;
  const hasBubble = !user || !!m.text || inlineAttachments.length > 0;
  return <article className={`message ${m.role} ${informational ? 'command-result' : ''} ${pending(m.data?.status) ? 'streaming' : ''}`} aria-label={user ? '你的消息' : m.role === 'system' ? informational ? '命令结果' : '运行提示' : undefined}>
    {images.length ? <div className="message-user-images" aria-label="消息图片">{images.map(a => <span className="message-image-attachment" key={a.path} data-resource-path={a.path}><AttachmentImage attachment={{ name:a.name, path:a.path! }}/></span>)}</div> : null}
    {user ? <MediaList media={media}/> : null}
    {hasBubble ? <div className="message-content">
      {m.role === 'system' ? <div className="system-message-label">{informational ? m.kind === 'goalState' ? '目标' : '命令结果' : '运行提示'}</div> : null}
      {inlineAttachments.length ? <div className="message-attachments">{inlineAttachments.map((a,i) => a.kind === 'image' && a.path ? <span className="message-image-attachment" key={a.path}><AttachmentImage attachment={{ name:a.name, path:a.path }}/></span> : <span key={i}><FileCode2 size={12}/>{a.name}</span>)}</div> : null}
      {user ? m.text ? <div className="message-text">{m.text}</div> : null : <Suspense fallback={<div className="message-text">{m.text}</div>}><Markdown text={m.text} openFile={openFile} streaming={m.role === 'assistant' && pending(m.data?.status)} /></Suspense>}
      {user ? null : <MediaList media={media}/>}
    </div> : null}
    {user && m.text ? <CopyButton className="copy-message user-copy" text={m.text} label="复制消息" iconOnly /> : null}
  </article>;
});

function TurnProgress({ label }: { label: string }) {
  return <div className="working-indicator" role="status" aria-label="任务运行状态"><span className="pending-text">{label}</span></div>;
}

function TurnView({ turn, lifecycle, pendingLabel, agentName, autoExpand, processDisplay, showDiff, openFile, onPlanAction }: { turn: ConversationTurn; lifecycle: TurnLifecycle; pendingLabel: string; agentName: string; autoExpand: boolean; processDisplay: ProcessDisplay; showDiff:(name:string,text:string)=>void; openFile:(path:string,line?:number)=>void; onPlanAction?:(message:Message,decision:PlanDecision,feedback:string)=>Promise<void> }) {
  const [allChanges, setAllChanges] = useState(false);
  const [expandedProcess, setExpandedProcess] = useState<string | null>(null);
  const presentation = presentTurn(turn, lifecycle);
  const noticeIds = new Set(presentation.notices.map(message => message.id));
  const agentRow = (message:Message) => isSubagent(message) && collectSubagents([message]).length > 0;
  const process = presentation.process.filter(message=>!agentRow(message));
  const resultIds = new Set(presentation.final.map(message=>message.id));
  const result = turn.items.filter(message=>resultIds.has(message.id)||agentRow(message));
  const marker = turn.run;
  const processKey = marker?.id ?? turn.id;
  useEffect(() => { setExpandedProcess(null); }, [processKey, presentation.completed]);
  if (!turn.user && turn.items.length === 1 && turn.items[0].kind === 'modelSwitch') return <MessageView message={turn.items[0]} openFile={openFile} />;
  const completed = presentation.settled;
  const generating = !completed && (lifecycle.sessionStatus
    ? lifecycle.latest && isActive(lifecycle.sessionStatus)
    : pending(marker?.data?.status) || turn.items.some(message => pending(activityStatus(message))));
  const latestPlan = turn.items.filter(message=>message.kind === 'plan').at(-1);
  const processOpen = processDisplay === 'timeline' || expandedProcess === processKey;
  const reply = (presentation.completed ? presentation.final : turn.items).filter(m => m.role === 'assistant' && m.text).map(m => m.text).join('\n\n');
  const diff = turn.items.filter(m=>m.kind==='turnDiff').map(m=>m.text).join('\n');
  const files = [...new Map(diffFiles(diff).map(f=>[f.path,f])).values()];
  const added = files.reduce((count, file) => count + file.added, 0);
  const removed = files.reduce((count, file) => count + file.removed, 0);
  const resultText = marker?.data?.status === 'interrupted' ? '已停止' : marker?.data?.status === 'failed' ? '运行失败' : '';
  const summary = <>{marker?.data?.durationMs != null ? <span>用时 {duration(Number(marker.data.durationMs))}</span> : <span>{resultText || '执行过程'}</span>}{resultText && marker?.data?.durationMs != null ? <span>· {resultText}</span> : null}</>;
  function renderItems(items: Message[], live = false) {
    const blocks: (Message | Message[])[] = [];
    for (const item of items) {
      if (item.kind === 'turnDiff' || item.role === 'system' && !noticeIds.has(item.id) || item.kind === 'goalState' && !item.data?.goal || item.role === 'assistant' && !item.text && !messageMedia(item).length) continue;
      const previous = blocks[blocks.length - 1];
      if (isSubagent(item) && collectSubagents([item]).length) blocks.push(item);
      else if (isActivity(item)) { if (Array.isArray(previous)) previous.push(item); else blocks.push([item]); }
      else blocks.push(item);
    }
    const waiting = live && lifecycle.sessionStatus === 'waiting';
    const content = blocks.map((block, index) => Array.isArray(block) ? <ActivityGroup key={block[0].id} messages={block} autoExpand={autoExpand} processDisplay={processDisplay} live={live && !waiting && index === blocks.length - 1} /> : isSubagent(block) && collectSubagents([block]).length ? <SubagentRows key={block.id} messageId={block.id} /> : block.kind === 'plan' ? <Suspense key={block.id} fallback={<span className="pending-text">正在加载计划文档…</span>}><ProposedPlan text={block.text} streaming={!!generating && pending(block.data?.status)} openFile={openFile} respond={presentation.completed && lifecycle.latest && block.id === latestPlan?.id && onPlanAction ? (decision,feedback)=>onPlanAction(block,decision,feedback) : undefined}/></Suspense> : <MessageView key={block.id} message={block} openFile={openFile} />);
    if (waiting) content.push(<TurnProgress key="turn-progress" label={pendingLabel} />);
    else if (live && !Array.isArray(blocks.at(-1))) {
      const activities = items.filter(item => isActivity(item) && !isSubagent(item));
      content.push(<TurnProgress key="turn-progress" label={activities.length ? summarizeActivities(activities, true).label : pendingLabel} />);
    }
    return content;
  }
  return <div className="conversation-turn" data-conversation-turn={turn.id}>
    {turn.user ? <MessageView message={turn.user} openFile={openFile} /> : null}
    {turn.items.length || marker || generating ? <section className="assistant-turn" aria-label={`${agentName} 回复`}>
      {presentation.completed ? <div className="turn-process-header">
          {process.length && processDisplay === 'collapsible' ? <button type="button" className="turn-process-toggle" aria-label="执行过程" aria-expanded={processOpen} aria-controls={`process-${turn.id}`} onClick={() => setExpandedProcess(processOpen ? null : processKey)}>{summary}<ChevronRight size={15} className={processOpen ? 'expanded' : ''}/></button> : <div className="turn-process-label">{summary}</div>}
        </div> : null}
      {presentation.completed && processOpen ? <div className="turn-process-content" id={`process-${turn.id}`} role="region" aria-label="执行过程详情">{renderItems(process)}</div> : null}
      {presentation.completed ? renderItems(presentation.notices) : null}
      <div className={presentation.completed ? 'turn-final-result' : 'turn-live-content'}>{renderItems(presentation.completed ? result : turn.items, generating)}</div>
      {!generating && (reply || completed && !presentation.completed) ? <div className="turn-footer">{completed && !presentation.completed ? <div className={`turn-summary ${marker?.data?.status}`}>{summary}</div> : null}{reply ? <CopyButton className="turn-copy" text={reply} label="复制回复" iconOnly /> : null}</div> : null}
      {completed && files.length ? <section className="turn-changes" aria-label="本轮文件变更"><div className="turn-changes-heading"><span className="turn-changes-icon"><FileDiff size={22}/></span><div className="turn-changes-title"><strong>已编辑 {files.length} 个文件</strong><div className="turn-change-totals"><span className="added-count">+{added}</span><span className="removed-count">−{removed}</span></div></div><button className="view-changes" onClick={()=>showDiff('本轮变更',diff)}>查看变更</button></div><div className="turn-changes-files">{(allChanges ? files : files.slice(0, 3)).map(f => {
        const path = f.path.replace(/\\/g, '/');
        const split = path.lastIndexOf('/') + 1;
        return <button className="turn-change-file" key={f.path} title={f.path} data-resource-path={f.path} aria-label={`查看 ${f.path} 的变更，新增 ${f.added} 行，删除 ${f.removed} 行`} onClick={()=>showDiff(`本轮变更 · ${f.path}`,f.diff)}><span className="turn-change-path"><span>{path.slice(0,split)}</span>{path.slice(split)}</span><span className="turn-change-counts"><span className="added-count">+{f.added}</span><span className="removed-count">−{f.removed}</span></span></button>;
      })}</div>{files.length > 3 ? <button className="changes-toggle" aria-expanded={allChanges} onClick={()=>setAllChanges(value=>!value)}>{allChanges ? '收起文件' : `再显示 ${files.length - 3} 个文件`}<ChevronDown size={14} className={allChanges ? 'expanded' : ''}/></button> : null}</section> : null}
    </section> : null}
  </div>;
}

export const Conversation = memo(function Conversation({ messages, agentName, autoExpand, processDisplay = 'collapsible', showDiff, openFile, openPath, projectId, sessionId, sessionStatus, activeTurnId, openSubagent, onPlanAction, pendingLabel = sessionStatus === 'starting' ? '正在启动 Agent…' : sessionStatus === 'waiting' ? '等待你的确认…' : '正在思考…' }: { messages: Message[]; agentName: string; autoExpand: boolean; processDisplay?: ProcessDisplay; showDiff:(name:string,text:string)=>void; openFile:(path:string,line?:number)=>void; openPath?: OpenPath; projectId?: string; sessionId?: string; sessionStatus?: string; activeTurnId?: string | null; openSubagent?:(key:string)=>void; onPlanAction?:(message:Message,decision:PlanDecision,feedback:string)=>Promise<void>; pendingLabel?: string }) {
  const turns = conversationTurns(messages);
  const mediaContext = useMemo(() => ({ projectId, sessionId, openFile }), [projectId, sessionId, openFile]);
  const subagents = useMemo(() => ({ agents:collectSubagents(messages), open:openSubagent }), [messages, openSubagent]);
  return <SubagentContext.Provider value={subagents}><MediaContext.Provider value={mediaContext}><ResourceMenu openFile={openFile} openPath={openPath}><ConversationIndex turns={turns}/>{turns.map((turn, index) => <TurnView key={turn.id} turn={turn} lifecycle={{ sessionStatus, activeTurnId, latest: index === turns.length - 1 }} pendingLabel={pendingLabel} agentName={agentName} autoExpand={autoExpand} processDisplay={processDisplay} showDiff={showDiff} openFile={openFile} onPlanAction={onPlanAction} />)}{!turns.length && isActive(sessionStatus) ? <TurnProgress label={pendingLabel} /> : null}</ResourceMenu></MediaContext.Provider></SubagentContext.Provider>;
});

export function JumpToLatest({ onClick }: { onClick: () => void }) { return <button className="jump-to-latest" onClick={onClick}><ArrowDown size={14} />回到最新消息</button>; }
