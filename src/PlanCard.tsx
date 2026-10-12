import { useContext, useLayoutEffect, useRef, useState } from 'react';
import { Lightbulb, PanelRightOpen } from 'lucide-react';
import { PlanDocument } from './PlanDocument';
import { CopyButton } from './CopyButton';
import { PlanDisplayContext, planTitle } from './PlanDisplayContext';

export function PlanCard({ text, streaming = false, status, openFile }: { text: string; streaming?: boolean; status?: string; openFile?: (path: string, line?: number) => void }) {
  const { openPlan } = useContext(PlanDisplayContext);
  const preview = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [truncated, setTruncated] = useState(false);
  useLayoutEffect(() => {
    const measure = () => setTruncated(!!preview.current && preview.current.scrollHeight > preview.current.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    if (preview.current) observer.observe(preview.current);
    if (content.current) observer.observe(content.current);
    return () => observer.disconnect();
  }, []);
  const show = openPlan && !streaming && text.trim() ? () => openPlan(planTitle(text), text, openFile) : undefined;
  return <section className={`plan-card${streaming ? ' is-generating' : ''}`} aria-label={streaming ? '正在生成计划' : '计划文档'}>
    <header className="plan-card-header">
      <div><Lightbulb size={16} aria-hidden="true"/><span className={streaming ? 'pending-text' : undefined}>计划</span>{status ? <small role="status">{status}</small> : null}</div>
      <div className="plan-card-actions">
        {!streaming ? <CopyButton iconOnly className="plan-card-copy" text={text} label="复制完整计划"/> : null}
        {show ? <button type="button" className="icon-button" aria-label="在侧边栏查看完整计划" title="打开完整计划" onClick={show}><PanelRightOpen size={14}/></button> : null}
      </div>
    </header>
    <div ref={preview} className={`plan-card-preview${truncated ? ' is-truncated' : ''}`} role={show ? 'button' : undefined} tabIndex={show ? 0 : undefined} aria-label={show ? `打开完整计划：${planTitle(text)}` : undefined} onClick={show} onKeyDown={event => {
      if (show && event.target === event.currentTarget && ['Enter', ' '].includes(event.key)) { event.preventDefault(); show(); }
    }}>
      <div ref={content} inert={streaming || !!show}><PlanDocument text={text} streaming={streaming} openFile={openFile} preview/></div>
    </div>
  </section>;
}
