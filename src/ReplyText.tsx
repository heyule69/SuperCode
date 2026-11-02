import { createContext, lazy, Suspense, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { desktop } from './api';
import { MediaContext } from './Media';
import { CopyButton } from './CopyButton';
import { maskTranslation, restoreTranslation, translationParts } from './replyTranslation';
import { translationQueue, type TranslationState } from './translationQueue';
import './reply-translation.css';

const Markdown = lazy(() => import('./Markdown'));
export const ReplyTranslationContext = createContext(false);
const visibilityListeners = new Set<() => void>();
const notifyVisibility = () => { for (const listener of visibilityListeners) listener(); };
function watchVisibility(listener: () => void) {
  visibilityListeners.add(listener);
  if (visibilityListeners.size === 1) document.addEventListener('visibilitychange', notifyVisibility);
  return () => {
    visibilityListeners.delete(listener);
    if (!visibilityListeners.size) document.removeEventListener('visibilitychange', notifyVisibility);
  };
}

export function ReplyText({ text, streaming = false, openFile, renderText }: { text: string; streaming?: boolean; openFile?: (path: string, line?: number) => void; renderText?: (displayed: string) => ReactNode }) {
  const preference = useContext(ReplyTranslationContext);
  const { sessionId } = useContext(MediaContext);
  const prepared = useMemo(() => desktop && preference && text.length <= 256 * 1024 ? maskTranslation(text) : null, [text, preference]);
  const parts = useMemo(() => prepared ? translationParts(prepared.masked, streaming) : [], [prepared, streaming]);
  const enabled = desktop && preference && !!sessionId && parts.some(part => part.translate);
  const scope = sessionId ?? '';
  const [results, setResults] = useState<{ session?: string; values: Map<string, TranslationState> }>({ values: new Map() });
  const [originalScope, setOriginalScope] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [retryRevision, setRetryRevision] = useState(0);
  const element = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!enabled || !element.current) { setVisible(false); return; }
    let intersecting = typeof IntersectionObserver === 'undefined';
    const update = () => setVisible(intersecting && document.visibilityState !== 'hidden');
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      intersecting = entries.some(entry => entry.isIntersecting);
      update();
    });
    observer?.observe(element.current);
    const off = watchVisibility(update);
    update();
    return () => { observer?.disconnect(); off(); };
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !visible || !sessionId) return;
    const releases: (() => void)[] = [];
    const timer = window.setTimeout(() => {
      for (const part of parts) if (part.translate) releases.push(translationQueue.watch(sessionId, part.text, state => {
        setResults(previous => {
          const values = new Map(previous.session === sessionId ? previous.values : []);
          values.set(part.text, state);
          for (const key of values.keys()) if (!parts.some(part => part.text === key)) values.delete(key);
          return { session: sessionId, values };
        });
      }));
    }, 700);
    return () => { window.clearTimeout(timer); releases.forEach(release => release()); };
  }, [enabled, visible, sessionId, parts, retryRevision]);

  const states = results.session === sessionId ? results.values : new Map<string, TranslationState>();
  const hasTranslation = parts.some(part => part.translate && states.get(part.text)?.status === 'ready');
  const failures = parts.filter(part => part.translate && states.get(part.text)?.status === 'error');
  const pending = parts.some(part => part.translate && !['ready', 'error'].includes(states.get(part.text)?.status ?? ''));
  const original = originalScope === scope;
  const translated = prepared ? restoreTranslation(parts.map(part => part.translate && states.get(part.text)?.status === 'ready' ? states.get(part.text)!.text : part.text).join(''), prepared.values) : text;
  const displayed = enabled && hasTranslation && !original ? translated : text;

  return <div className="reply-text" ref={element}>
    {renderText ? renderText(displayed) : <Suspense fallback={<div className="message-text">{displayed}</div>}><Markdown text={displayed} openFile={openFile} streaming={streaming && (!hasTranslation || original)} /></Suspense>}
    {enabled && parts.some(part => part.translate) ? <div className="reply-translation" aria-label="回复翻译">
      <span role="status">{original ? '原文' : failures.length ? '翻译暂不可用，已保留原文' : pending ? '正在译成中文…' : '中文译文'}</span>
      {hasTranslation ? <button type="button" className="text-button" aria-pressed={original} onClick={() => setOriginalScope(original ? null : scope)}>{original ? '显示中文' : '显示原文'}</button> : null}
      {failures.length ? <button type="button" className="text-button" title={states.get(failures[0].text)?.error} onClick={() => { failures.forEach(part => translationQueue.retry(sessionId!, part.text)); setRetryRevision(value => value + 1); }}>重试翻译</button> : null}
      {hasTranslation && !pending && !failures.length && !streaming && !original ? <CopyButton className="text-button" text={translated} label="复制译文" /> : null}
    </div> : null}
  </div>;
}
