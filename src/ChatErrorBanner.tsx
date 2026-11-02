import { useEffect, useRef, useState } from 'react';
import { ExternalLink, LoaderCircle, X } from 'lucide-react';
import { call, desktop } from './api';
import './chat-error-banner.css';

function MagpieOfflineBody() {
  const [opening, setOpening] = useState(false);
  const [requested, setRequested] = useState(false);
  const [launchError, setLaunchError] = useState('');
  const pending = useRef(false), active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);

  async function open() {
    if (pending.current || !desktop) return;
    pending.current = true; setOpening(true); setLaunchError(''); setRequested(false);
    try {
      await call('open_magpie');
      if (active.current) setRequested(true);
    } catch (error) {
      if (active.current) setLaunchError(error instanceof Error ? error.message : String(error));
    } finally {
      pending.current = false;
      if (active.current) setOpening(false);
    }
  }

  return <div className="magpie-offline-body">
    <span>Magpie 尚未打开或网关未连接，请点击「打开 Magpie」后再发送。</span>
    <div className="magpie-offline-action">
      <button type="button" className="quiet-button" disabled={!desktop || opening} onClick={() => void open()}>
        {opening ? <LoaderCircle size={14} className="spin"/> : <ExternalLink size={14}/>}
        {opening ? '正在打开…' : '打开 Magpie'}
      </button>
      <span className="magpie-open-status" role="status">{requested ? '已请求打开，请等待网关就绪后再次发送。' : opening ? '正在请求打开 Magpie…' : '草稿已保留'}</span>
    </div>
    {launchError ? <span className="magpie-open-error">打开失败：{launchError}</span> : null}
  </div>;
}

export function ChatErrorBanner({ error, dismiss, dismissLabel = '关闭提示' }: { error: string; dismiss: () => void; dismissLabel?: string }) {
  const magpieOffline = /^Magpie 网关未连接[。，]/.test(error);
  return <div className={`error-banner${magpieOffline ? ' magpie-offline-banner' : ''}`} role="alert">
    {magpieOffline ? <MagpieOfflineBody key={error}/> : <span>{error}</span>}
    <button type="button" className="icon-button" title={dismissLabel} aria-label={dismissLabel} onClick={dismiss}><X size={15}/></button>
  </div>;
}
