import { useEffect, useRef, useState } from 'react';
import { Check, Copy, X } from 'lucide-react';

export function CopyButton({ text, label, iconOnly = false, className = '' }: { text: string; label: string; iconOnly?: boolean; className?: string }) {
  const [status, setStatus] = useState<'idle' | 'copied' | 'failed'>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; clearTimeout(timer.current); }; }, []);
  const feedback = status === 'copied' ? '已复制' : status === 'failed' ? '复制失败，请手动选择文本' : label;
  async function copy() {
    try { await navigator.clipboard.writeText(text); if (mounted.current) setStatus('copied'); }
    catch { if (mounted.current) setStatus('failed'); }
    if (mounted.current) { clearTimeout(timer.current); timer.current = setTimeout(() => setStatus('idle'), 2000); }
  }
  return <button type="button" className={`${iconOnly ? 'icon-button' : 'text-button'} ${className} ${status === 'failed' ? 'copy-failed' : ''}`} title={feedback} aria-label={feedback} onClick={() => void copy()}>
    {status === 'copied' ? <Check size={13} /> : status === 'failed' ? <X size={13} /> : <Copy size={13} />}
    {iconOnly ? <span className="sr-only" role="status">{status === 'idle' ? '' : feedback}</span> : <span role="status">{status === 'failed' ? '复制失败' : feedback}</span>}
  </button>;
}
