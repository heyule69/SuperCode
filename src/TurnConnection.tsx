import { useEffect, useState } from 'react';
import { CheckCircle2, Clock3, RefreshCw, Unplug, XCircle } from 'lucide-react';
import { readRunConnection } from './runConnection';
import './turn-connection.css';

const titles = { network:'正在重新连接模型', rateLimit:'请求受限，正在重试', server:'模型服务暂不可用，正在重试', auth:'模型认证失败，正在重试', billing:'额度或计费异常，正在重试', unknown:'正在重试模型请求' };

export function TurnConnection({ active, connection: raw, lastProgressAt, waitingForTool = false, starting = false, normalLabel }: { active: boolean; connection?: unknown; lastProgressAt?: number; waitingForTool?: boolean; starting?: boolean; normalLabel?: string }) {
  const [openedAt] = useState(Date.now);
  const [now, setNow] = useState(Date.now);
  const connection = readRunConnection(raw);
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  const progressAt = typeof lastProgressAt === 'number' && Number.isFinite(lastProgressAt) && lastProgressAt > 0 && lastProgressAt <= now + 60000 ? lastProgressAt : openedAt;
  const quietSeconds = Math.max(0, Math.floor((now - progressAt) / 1000));
  const restored = active && connection?.status === 'restored' && now - connection.at < 5000;
  const retrying = active && connection?.status === 'retrying';
  const failed = connection?.status === 'failed', stopped = connection?.status === 'stopped';
  const disconnected = connection?.status === 'disconnected';
  const quiet = active && quietSeconds >= 30 && !retrying && !failed && !restored && !disconnected && !stopped;
  if (!retrying && !restored && !failed && !stopped && !disconnected && !quiet) return normalLabel ? <div className="working-indicator" role="status" aria-label="任务运行状态"><span className="pending-text">{normalLabel}</span></div> : null;
  const mode = retrying ? 'retrying' : restored ? 'restored' : failed || disconnected ? 'failed' : stopped ? 'stopped' : 'quiet';
  const Icon = retrying ? RefreshCw : restored ? CheckCircle2 : failed || disconnected ? Unplug : stopped ? XCircle : Clock3;
  const title = retrying ? titles[connection!.kind] : restored ? '连接已恢复' : disconnected ? '与 Agent 的连接已断开' : failed ? '重试未能恢复连接' : stopped ? '已停止重试' : waitingForTool ? '工具仍在运行，暂未返回新进展' : starting ? '正在等待 Agent 启动' : '正在等待模型响应';
  const seconds = connection?.retryAt == null ? undefined : Math.max(0, Math.ceil((connection.retryAt - Math.max(now, connection.at)) / 1000));
  return <aside className={`turn-connection ${mode}`} aria-label="模型连接状态">
    <Icon size={16} className={retrying ? 'connection-spinner' : undefined} aria-hidden="true" />
    <div className="connection-content"><div className="connection-heading"><strong role="status">{title}</strong>{retrying && connection?.attempt ? <span className="connection-attempt">第 {connection.attempt}{connection.maxAttempts ? ` / ${connection.maxAttempts}` : ''} 次</span> : null}</div>
      <p>{retrying ? <><span aria-live="off">{seconds != null && seconds > 0 ? `预计 ${seconds} 秒后重试。` : '正在等待重试结果。'}</span>任务仍在本轮继续，可随时停止。</> : restored ? '已重新收到响应，将继续当前任务。' : disconnected ? '本轮已停止。请检查连接后重新发送消息。' : failed ? '本轮未能恢复，请检查网络、模型连接或额度。' : stopped ? '本轮任务已停止。' : <>已 {quietSeconds} 秒未收到新进展。{waitingForTool ? '这不代表连接已断开，可等待工具完成或停止任务。' : '尚未收到重连通知，当前无法确认是否正在重连。'}</>}</p>
    </div>
  </aside>;
}
