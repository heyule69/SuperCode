import { useEffect, useState } from 'react';
import { CheckCircle2, RefreshCw, Unplug, XCircle } from 'lucide-react';
import { readRunConnection } from './runConnection';
import './turn-connection.css';

const titles = { network:'正在重新连接模型', rateLimit:'请求受限，正在重试', server:'模型服务暂不可用，正在重试', auth:'模型认证失败，正在重试', billing:'额度或计费异常，正在重试', unknown:'正在重试模型请求' };

export function TurnConnection({ active, connection: raw, normalLabel }: { active: boolean; connection?: unknown; normalLabel?: string }) {
  const [clock, setClock] = useState(Date.now);
  const now = Math.max(clock, Date.now());
  const connection = readRunConnection(raw);
  const status = connection?.status, at = connection?.at, retryAt = connection?.retryAt;
  useEffect(() => {
    if (!active) return;
    if (status === 'retrying' && retryAt != null && retryAt > Date.now()) {
      const timer = window.setInterval(() => {
        const next = Date.now(); setClock(next);
        if (next >= retryAt) window.clearInterval(timer);
      }, 1000);
      return () => window.clearInterval(timer);
    }
    if (status === 'restored' && at != null && at + 5000 > Date.now()) {
      const timer = window.setTimeout(() => setClock(Date.now()), at + 5000 - Date.now());
      return () => window.clearTimeout(timer);
    }
  }, [active, status, at, retryAt]);
  const restored = active && connection?.status === 'restored' && now - connection.at < 5000;
  const retrying = active && connection?.status === 'retrying';
  const failed = connection?.status === 'failed', stopped = connection?.status === 'stopped';
  const disconnected = connection?.status === 'disconnected';
  if (!retrying && !restored && !failed && !stopped && !disconnected) return normalLabel ? <div className="working-indicator" role="status" aria-label="任务运行状态"><span className="pending-text">{normalLabel}</span></div> : null;
  const mode = retrying ? 'retrying' : restored ? 'restored' : failed || disconnected ? 'failed' : 'stopped';
  const Icon = retrying ? RefreshCw : restored ? CheckCircle2 : failed || disconnected ? Unplug : XCircle;
  const title = retrying ? titles[connection!.kind] : restored ? '连接已恢复' : disconnected ? '与 Agent 的连接已断开' : failed ? '重试未能恢复连接' : '已停止重试';
  const seconds = connection?.retryAt == null ? undefined : Math.max(0, Math.ceil((connection.retryAt - Math.max(now, connection.at)) / 1000));
  return <aside className={`turn-connection ${mode}`} aria-label="模型连接状态">
    <Icon size={16} className={retrying ? 'connection-spinner' : undefined} aria-hidden="true" />
    <div className="connection-content"><div className="connection-heading"><strong role="status">{title}</strong>{retrying && connection?.attempt ? <span className="connection-attempt">第 {connection.attempt}{connection.maxAttempts ? ` / ${connection.maxAttempts}` : ''} 次</span> : null}</div>
      <p>{retrying ? <><span aria-live="off">{seconds != null && seconds > 0 ? `预计 ${seconds} 秒后重试。` : '正在等待重试结果。'}</span>任务仍在本轮继续，可随时停止。</> : restored ? '已重新收到响应，将继续当前任务。' : disconnected ? '本轮已停止。请检查连接后重新发送消息。' : failed ? '本轮未能恢复，请检查网络、模型连接或额度。' : '本轮任务已停止。'}</p>
    </div>
  </aside>;
}
