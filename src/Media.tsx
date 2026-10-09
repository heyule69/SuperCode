import { createContext, memo, useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ExternalLink, Image, LoaderCircle, Maximize2, Music2, Play, X } from 'lucide-react';
import { call, desktop } from './api';
import { mediaReference, type MediaReference } from './mediaSources';
import './media.css';

export const MediaContext = createContext<{ projectId?: string; openFile?: (path: string, line?: number) => void }>({});
interface PreparedMedia { token: string; previewToken: string; path: string; kind: MediaReference['kind']; name: string }

export const MediaCard = memo(function MediaCard({ media }: { media: MediaReference }) {
  const context = useContext(MediaContext);
  const target = useRef<HTMLSpanElement>(null);
  const imageButton = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(false);
  const [source, setSource] = useState('');
  const [fullSource, setFullSource] = useState('');
  const [error, setError] = useState('');
  const [expanded, setExpanded] = useState(false);
  const [playing, setPlaying] = useState(false);
  useEffect(() => {
    if (!target.current) return;
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: '120px' });
    observer.observe(target.current); return () => observer.disconnect();
  }, []);
  useEffect(() => {
    setSource(''); setFullSource(''); setError(''); setPlaying(false); setExpanded(false);
    if (!visible) return;
    let cancelled = false;
    if (/^https?:/i.test(media.path)) { setSource(media.path); setFullSource(media.path); return; }
    if (!desktop) { setError('本地媒体请在桌面版中查看'); return; }
    void call<PreparedMedia>('prepare_media', { path: media.path, projectId: context.projectId ?? null }).then(result => {
      if (cancelled) return;
      setSource(convertFileSrc(result.previewToken, 'supercode-media')); setFullSource(convertFileSrc(result.token, 'supercode-media'));
    }).catch(e => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [media.path, context.projectId, visible]);
  const Icon = media.kind === 'image' ? Image : media.kind === 'video' ? Play : Music2;
  const label = media.kind === 'image' ? '图片' : media.kind === 'video' ? '视频' : '音频';
  function openInSystem() {
    if (desktop) void call('open_media', { path: media.path, projectId: context.projectId ?? null }).catch(e => setError(String(e)));
    else context.openFile?.(media.path);
  }
  function activatePlayer(event: React.SyntheticEvent<HTMLMediaElement>) {
    // Only the player explicitly started by the user should remain audible.
    document.querySelectorAll<HTMLMediaElement>('.chat-media-player').forEach(player => { if (player !== event.currentTarget) player.pause(); });
  }
  const content = error ? <span className="chat-media-error" role="status"><Icon size={22}/><span>{error}</span></span>
    : !source ? <span className="chat-media-placeholder"><Icon size={24}/><span>{visible ? <><LoaderCircle size={14} className="spin"/>正在加载{label}…</> : label}</span></span>
    : media.kind === 'image' ? <button ref={imageButton} type="button" className="chat-image-button" onClick={() => setExpanded(true)} aria-label={`放大图片 ${media.name}`}><img src={source} alt={media.name} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setError('图片无法显示，文件可能已移动或格式不受支持')}/><span className="chat-image-zoom"><Maximize2 size={16}/></span></button>
    : media.kind === 'video' && !playing ? <button type="button" className="chat-video-start" onClick={() => setPlaying(true)} aria-label={`加载视频 ${media.name}`}><Play size={28}/><span>打开视频播放器</span></button>
    : media.kind === 'video' ? <video className="chat-media-player" src={fullSource} controls playsInline preload="metadata" onPlay={activatePlayer} onError={() => setError('当前播放器无法播放此视频，可在系统中打开')}/>
    : <audio className="chat-media-player" src={fullSource} controls preload="none" onPlay={activatePlayer} onError={() => setError('当前播放器无法播放此音频，可在系统中打开')}/>;
  return <span ref={target} className={`chat-media-card chat-media-${media.kind}`} data-media-source={media.path}>
    {content}<span className="chat-media-caption"><Icon size={14}/><span title={media.name}>{media.name}</span>{(desktop || context.openFile) && !/^https?:/i.test(media.path) ? <button type="button" className="icon-button" aria-label={`在系统中打开 ${media.name}`} onClick={openInSystem}><ExternalLink size={14}/></button> : null}</span>
    {expanded && fullSource ? <ImageViewer source={fullSource} name={media.name} close={() => { setExpanded(false); imageButton.current?.focus({ preventScroll: true }); }}/> : null}
  </span>;
});

function ImageViewer({ source, name, close }: { source: string; name: string; close: () => void }) {
  const dismiss = useRef<HTMLButtonElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    dismiss.current?.focus();
    const handle = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); close(); } else if (event.key === 'Tab') { event.preventDefault(); dismiss.current?.focus(); } };
    document.addEventListener('keydown', handle, true); return () => document.removeEventListener('keydown', handle, true);
  }, []);
  return createPortal(<div className="image-preview-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) close(); }}><section className="image-preview" role="dialog" aria-modal="true" aria-label={`图片预览 ${name}`}><header><span>{name}</span><button ref={dismiss} type="button" className="icon-button" aria-label="关闭图片预览" onClick={close}><X size={20}/></button></header><div className="image-preview-content">{failed ? <p role="status">原图无法显示</p> : <img src={source} alt={name} referrerPolicy="no-referrer" onError={() => setFailed(true)}/>}</div></section></div>, document.body);
}

export function MediaList({ media }: { media: MediaReference[] }) {
  return media.length ? <div className="chat-media-list">{media.map(ref => <MediaCard key={ref.path} media={ref}/>)}</div> : null;
}

export function MarkdownMedia({ src, alt }: { src?: string; alt?: string }) {
  const media = mediaReference(src ?? '', alt, 'image');
  if (!media) return <span className="chat-media-invalid">{alt || '媒体'} · 无法显示此来源</span>;
  // Markdown's image syntax is also the desktop client's media embed syntax.
  const extension = src?.match(/\.(mp4|m4v|webm|mov|mkv|mp3|wav|ogg|oga|flac|m4a)(?:[?#].*)?$/i)?.[1]?.toLowerCase();
  if (extension) media.kind = ['mp4', 'm4v', 'webm', 'mov', 'mkv'].includes(extension) ? 'video' : 'audio';
  return <MediaCard media={media}/>;
}
