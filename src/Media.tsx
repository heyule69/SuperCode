import { createContext, memo, useContext, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { convertFileSrc } from '@tauri-apps/api/core';
import { ChevronDown, ExternalLink, Image, Images, LoaderCircle, Maximize2, Music2, Play, X } from 'lucide-react';
import { call, desktop } from './api';
import { embeddedMedia, type MediaReference } from './mediaSources';
import { firstVideoFrame } from './videoPreview';
import './media.css';

export const MediaContext = createContext<{ projectId?: string; sessionId?: string; openFile?: (path: string, line?: number) => void }>({});
interface PreparedMedia { token: string; previewToken: string; path: string; kind: MediaReference['kind']; name: string }

export const MediaCard = memo(function MediaCard({ media, process = false }: { media: MediaReference; process?: boolean }) {
  const context = useContext(MediaContext);
  const target = useRef<HTMLSpanElement>(null);
  const imageButton = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(false);
  const [source, setSource] = useState('');
  const [fullSource, setFullSource] = useState('');
  const [error, setError] = useState('');
  const [expired, setExpired] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [poster, setPoster] = useState('');
  const [previewFailed, setPreviewFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!target.current) return;
    if (typeof IntersectionObserver === 'undefined') { setVisible(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setVisible(true); observer.disconnect(); } }, { rootMargin: '120px' });
    observer.observe(target.current); return () => observer.disconnect();
  }, []);
  useEffect(() => {
    setSource(''); setFullSource(''); setError(''); setExpired(false); setPlaying(false); setExpanded(false);
    if (!visible) return;
    let cancelled = false;
    if (/^https?:/i.test(media.path)) { setSource(media.path); setFullSource(media.path); return; }
    if (!desktop) { setError('本地媒体请在桌面版中查看'); return; }
    void call<PreparedMedia | {unavailable:'expired'}>('prepare_media', { path: media.path, projectId: context.projectId ?? null, sessionId: context.sessionId || null, ...(process?{processPreview:true}:{}) }).then(result => {
      if (cancelled) return;
      if ('unavailable' in result) {setExpired(true);return;}
      setSource(convertFileSrc(result.previewToken, 'supercode-media')); setFullSource(convertFileSrc(result.token, 'supercode-media'));
    }).catch(e => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [media.path, context.projectId, context.sessionId, visible, attempt, process]);
  useEffect(() => {
    if (playing) return;
    setPoster(''); setPreviewFailed(false);
    if (media.kind !== 'video' || !fullSource) return;
    const controller = new AbortController();
    void firstVideoFrame(fullSource, controller.signal).then(frame => {
      if (!controller.signal.aborted) setPoster(frame);
    }).catch(() => { if (!controller.signal.aborted) setPreviewFailed(true); });
    return () => controller.abort();
  }, [media.kind, fullSource, playing, attempt]);
  const Icon = media.kind === 'image' ? Image : media.kind === 'video' ? Play : Music2;
  const label = media.kind === 'image' ? '图片' : media.kind === 'video' ? '视频' : '音频';
  function openInSystem() {
    if (desktop) void call('open_media', { path: media.path, projectId: context.projectId ?? null, sessionId: context.sessionId || null }).catch(e => setError(String(e)));
    else context.openFile?.(media.path);
  }
  function activatePlayer(event: React.SyntheticEvent<HTMLMediaElement>) {
    // Only the player explicitly started by the user should remain audible.
    document.querySelectorAll<HTMLMediaElement>('.chat-media-player').forEach(player => { if (player !== event.currentTarget) player.pause(); });
  }
  const content = expired ? <span className="chat-media-expired" role="status"><Image size={18}/><span>过程截图已自动清理</span></span>
    : error ? <span className="chat-media-error" role="status"><Icon size={22}/><span>{error}</span><button type="button" className="quiet-button" onClick={() => setAttempt(value => value + 1)}>重新加载{label}</button></span>
    : !source ? <span className="chat-media-placeholder"><Icon size={24}/><span>{visible ? <><LoaderCircle size={14} className="spin"/>正在加载{label}…</> : label}</span></span>
    : media.kind === 'image' ? <button ref={imageButton} type="button" className="chat-image-button" onClick={() => setExpanded(true)} aria-label={`放大图片 ${media.name}`}><img src={source} alt={media.name} loading="lazy" decoding="async" referrerPolicy="no-referrer" onError={() => setError('图片无法显示，文件可能已移动或格式不受支持')}/><span className="chat-image-zoom"><Maximize2 size={16}/></span></button>
    : media.kind === 'video' && !playing ? <button type="button" className={`chat-video-start${poster ? ' has-poster' : ''}`} onClick={() => setPlaying(true)} aria-label={`加载视频 ${media.name}`}>{poster ? <img className="chat-video-poster" src={poster} alt={`${media.name} 首帧`} decoding="async"/> : null}<span className="chat-video-overlay"><Play size={28}/>{poster ? null : <span>{previewFailed ? '首帧暂不可用，点击打开播放器' : <><LoaderCircle size={14} className="spin"/>正在加载视频首帧…</>}</span>}</span></button>
    : media.kind === 'video' ? <video className="chat-media-player" src={fullSource} poster={poster || undefined} controls playsInline preload="metadata" onPlay={activatePlayer} onError={() => setError('当前播放器无法播放此视频，可在系统中打开')}/>
    : <audio className="chat-media-player" src={fullSource} controls preload="none" onPlay={activatePlayer} onError={() => setError('当前播放器无法播放此音频，可在系统中打开')}/>;
  return <span ref={target} className={`chat-media-card chat-media-${media.kind}${process?' chat-media-process':''}`} data-media-source={expired?undefined:media.path} tabIndex={-1}>
    {content}<span className="chat-media-caption"><Icon size={14}/><span title={media.name}>{media.name}</span>{!expired && (desktop || context.openFile) && !/^https?:/i.test(media.path) ? <button type="button" className="icon-button" aria-label={`在系统中打开 ${media.name}`} onClick={openInSystem}><ExternalLink size={14}/></button> : null}</span>
    {expanded && fullSource ? <ImageViewer source={fullSource} path={media.path} name={media.name} close={() => { setExpanded(false); imageButton.current?.focus({ preventScroll: true }); }}/> : null}
  </span>;
});

function ImageViewer({ source, path, name, close }: { source: string; path: string; name: string; close: () => void }) {
  const dismiss = useRef<HTMLButtonElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    dismiss.current?.focus({preventScroll:true});
    window.dispatchEvent(new Event('supercode-media-overlay'));
    const handle = (event: KeyboardEvent) => { if (document.querySelector('.resource-context-menu')) return; if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); close(); } else if (event.key === 'Tab') { event.preventDefault(); dismiss.current?.focus({preventScroll:true}); } };
    document.addEventListener('keydown', handle, true); return () => {document.removeEventListener('keydown', handle, true);window.dispatchEvent(new Event('supercode-media-overlay'));};
  }, []);
  return createPortal(<div className="image-preview-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) close(); }}><section className="image-preview" role="dialog" aria-modal="true" aria-label={`图片预览 ${name}`} data-media-source={path} tabIndex={-1}><header><span>{name}</span><button ref={dismiss} type="button" className="icon-button" aria-label="关闭图片预览" onClick={close}><X size={20}/></button></header><div className="image-preview-content">{failed ? <p role="status">原图无法显示</p> : <img src={source} alt={name} referrerPolicy="no-referrer" onError={() => setFailed(true)}/>}</div></section></div>, document.body);
}

export function MediaList({ media, process=false }: { media: MediaReference[]; process?: boolean }) {
  const groups: (MediaReference | MediaReference[])[] = [];
  for (const ref of media) {
    const last = groups[groups.length - 1];
    if (ref.kind === 'image') { if (Array.isArray(last)) last.push(ref); else groups.push([ref]); }
    else groups.push(ref);
  }
  return media.length ? <div className="chat-media-list">{groups.map(group => Array.isArray(group)
    ? <div className="chat-image-gallery" role="group" aria-label="图片组" data-image-count={group.length} key={`images-${group[0].path}`}>{group.map(ref => <MediaCard key={ref.path} media={ref} process={process}/>)}</div>
    : <MediaCard key={group.path} media={group} process={process}/>)}</div> : null;
}

export function ProcessMedia({media}:{media:MediaReference[]}) {
  const [pageEnd,setPageEnd]=useState<number|null>(null);
  const images=media.filter(item=>item.kind==='image');
  const other=media.filter(item=>item.kind!=='image').slice(0,12);
  const end=Math.min(pageEnd??images.length,images.length), start=Math.max(0,end-12);
  const expanded=pageEnd!==null;
  return <>
    {images.length ? <div className="process-media">
      <button type="button" className="process-media-summary" aria-expanded={expanded} onClick={()=>setPageEnd(value=>value===null?images.length:null)}>
        <Images size={15}/><span>已查看 {images.length} 张图像</span><ChevronDown size={13} className={expanded?'expanded':''}/>
      </button>
      {expanded ? <div className="process-media-gallery">
        <MediaList media={images.slice(start,end)} process/>
        {images.length>12 ? <div className="process-media-pages">
          <button type="button" disabled={start===0} onClick={()=>setPageEnd(start)}>更早图像</button>
          <span>{start+1}–{end} / {images.length}</span>
          <button type="button" disabled={end===images.length} onClick={()=>setPageEnd(Math.min(images.length,end+12))}>较新图像</button>
        </div> : null}
      </div> : null}
    </div> : null}
    <MediaList media={other}/>
  </>;
}

export function MarkdownMedia({ src, alt }: { src?: string; alt?: string }) {
  const media = embeddedMedia(src ?? '', alt);
  if (!media) return <span className="chat-media-invalid">{alt || '媒体'} · 无法显示此来源</span>;
  return <MediaCard media={media}/>;
}
