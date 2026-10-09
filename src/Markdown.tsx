import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Children, createContext, isValidElement, useContext, useEffect, useMemo, useState, type ComponentProps, type CSSProperties, type ReactNode } from 'react';
import type { ExtraProps } from 'react-markdown';
import type { Root } from 'hast';
import { CopyButton } from './CopyButton';
import { describeLink, resourceFromText } from './links';
import { call, desktop } from './api';
import { StreamReveal } from './streamReveal';
import { FileText, Globe, Image as ImageIcon } from 'lucide-react';
import { MarkdownMedia, MediaList } from './Media';
import { referencedMedia } from './mediaSources';

function RevealSpan({ node, children, ...props }: ComponentProps<'span'> & ExtraProps) {
  const at = Number(node?.properties['data-stream-at']);
  const fragmentKey = String(node?.properties['data-stream-key'] ?? '');
  // A Markdown structure change can remount a span. Continue at its original time.
  const delay = useMemo(() => Number.isFinite(at) ? at - performance.now() : 0, [at, fragmentKey]);
  return <span {...props} key={fragmentKey} style={{ '--reveal-delay': `${delay}ms` } as CSSProperties}>{children}</span>;
}

function contentText(node: ReactNode): string { return Children.toArray(node).map(v => typeof v === 'string' || typeof v === 'number' ? String(v) : isValidElement<{ children?: ReactNode }>(v) ? contentText(v.props.children) : '').join(''); }
function CodeBlock({ children }: { children?: ReactNode }) {
  const code = Children.toArray(children).find(isValidElement) as React.ReactElement<{ className?: string }> | undefined;
  const language = code?.props.className?.replace('language-', '') ?? '代码';
  return <div className="code-block"><div className="code-block-heading"><span>{language}</span><CopyButton label="复制代码" text={contentText(children).replace(/\n$/, '')} /></div><pre>{children}</pre></div>;
}

const MarkdownActions = createContext<{ openFile?: (path: string, line?: number) => void; report: (error: string) => void }>({ report: () => {} });

function MarkdownCode({ children, className, node: _node, ...props }: ComponentProps<'code'> & ExtraProps) {
  const { openFile } = useContext(MarkdownActions);
  const value = contentText(children);
  const link = !className && !value.includes('\n') ? resourceFromText(value) : null;
  if (link?.kind === 'file') return <a className="markdown-file-link" href={link.target} data-resource-path={value} title={link.target} onClick={event => { event.preventDefault(); openFile?.(link.target, link.line); }}><code {...props}>{children}</code></a>;
  return <code {...props} className={className}>{children}</code>;
}

function MarkdownLink({ href, children }: ComponentProps<'a'> & ExtraProps) {
  const { openFile, report } = useContext(MarkdownActions);
  const link = describeLink(href ?? '');
  if (link.kind === 'unsupported') return <span>{children}</span>;
  const Icon = link.kind === 'external' ? Globe : /\.(png|jpe?g|gif|webp|svg|bmp)$/i.test(link.target) ? ImageIcon : FileText;
  return <a className={`markdown-link ${link.kind === 'anchor' ? 'markdown-anchor' : ''}`} href={href} title={link.target} target={link.kind === 'external' ? '_blank' : undefined} rel="noopener noreferrer" onClick={event => {
    if (link.kind === 'file') { event.preventDefault(); openFile?.(link.target, link.line); }
    else if (link.kind === 'external' && desktop) { event.preventDefault(); report(''); void call('open_external_link', { url: link.target }).catch(error => report(String(error))); }
  }}>{link.kind !== 'anchor' ? <Icon size={14} aria-hidden="true"/> : null}{children}</a>;
}

// Stable component types let React update text in place. Inline component
// functions remounted links/code on every stream delta and animation timeout.
const components = { span: RevealSpan, pre: CodeBlock, code: MarkdownCode, a: MarkdownLink, img: MarkdownMedia };
const safeUrl = (href: string) => describeLink(href).kind === 'unsupported' ? '' : href;

export default function Markdown({ text, openFile, streaming = false }: { text: string; openFile?: (path: string, line?: number) => void; streaming?: boolean }) {
  const [error, setError] = useState('');
  const actions = useMemo(() => ({ openFile, report: setError }), [openFile]);
  const [reveal] = useState(() => new StreamReveal());
  const [settled, setSettled] = useState(0);
  reveal.update(text, streaming, performance.now());
  const expires = reveal.expiresAt;
  useEffect(() => {
    if (!expires) return;
    const timer = window.setTimeout(() => setSettled(value => value + 1), Math.max(0, expires - performance.now()) + 10);
    return () => window.clearTimeout(timer);
  }, [expires]);
  const plugins = useMemo(() => [() => (root: Root) => reveal.decorate(root)], [reveal, text, streaming, settled]);
  const media = useMemo(() => referencedMedia(text), [text]);
  return <MarkdownActions.Provider value={actions}><ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={plugins} urlTransform={safeUrl} components={components}>{text}</ReactMarkdown><MediaList media={media}/>{error ? <p className="inline-error" role="alert">{error}</p> : null}</MarkdownActions.Provider>;
}
