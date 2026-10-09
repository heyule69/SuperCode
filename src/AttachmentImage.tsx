import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Image, LoaderCircle, X } from 'lucide-react';
import { call, desktop } from './api';
import type { Attachment } from './attachments';
import './attachments.css';

export function AttachmentImage({ attachment }: { attachment: Pick<Attachment, 'path' | 'name'> }) {
  const target = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(false);
  const [preview, setPreview] = useState('');
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!target.current) return;
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: '160px' });
    observer.observe(target.current); return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!visible) return;
    let cancelled = false; setPreview(''); setFailed(false);
    if (!desktop) { setFailed(true); return; }
    void call<string>('read_image_attachment', { path: attachment.path, full: false }).then(image => { if (!cancelled) setPreview(image); }).catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [attachment.path, visible]);
  return <><button ref={target} type="button" className="attachment-image" aria-label={`查看图片 ${attachment.name}`} title={failed ? `${attachment.name} · 图片不存在或无法预览` : attachment.name} onClick={() => setOpen(true)}>
    {preview ? <img src={preview} alt={attachment.name} decoding="async" onError={() => { setPreview(''); setFailed(true); }}/> : <span className="attachment-image-placeholder"><Image size={20}/><small>{failed ? '无法预览' : '图片'}</small></span>}
  </button>{open ? <ImagePreview attachment={attachment} close={() => { setOpen(false); target.current?.focus({ preventScroll: true }); }}/> : null}</>;
}

function ImagePreview({ attachment, close }: { attachment: Pick<Attachment, 'path' | 'name'>; close: () => void }) {
  const [source, setSource] = useState(''); const [error, setError] = useState(''); const dismiss = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    let cancelled = false; dismiss.current?.focus();
    void call<string>('read_image_attachment', { path: attachment.path, full: true }).then(image => { if (!cancelled) setSource(image); }).catch(e => { if (!cancelled) setError(String(e)); });
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); close(); } else if (event.key === 'Tab') { event.preventDefault(); dismiss.current?.focus(); } };
    document.addEventListener('keydown', escape, true);
    return () => { cancelled = true; document.removeEventListener('keydown', escape, true); };
  }, [attachment.path]);
  return createPortal(<div className="image-preview-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) close(); }}><section className="image-preview" role="dialog" aria-modal="true" aria-label={`图片预览 ${attachment.name}`}><header><span>{attachment.name}</span><button ref={dismiss} type="button" className="icon-button" aria-label="关闭图片预览" onClick={close}><X size={20}/></button></header><div className="image-preview-content">{source ? <img src={source} alt={attachment.name} onError={() => { setSource(''); setError('无法显示这张图片'); }}/> : error ? <p role="alert">{error}</p> : <span role="status"><LoaderCircle size={18} className="spin"/>正在打开图片…</span>}</div></section></div>, document.body);
}
