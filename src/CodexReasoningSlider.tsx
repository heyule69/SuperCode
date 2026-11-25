import type { CSSProperties } from 'react';
import { ChevronRight, RotateCcw, Zap } from 'lucide-react';
import { effortLabels, visibleCodexEfforts } from './modelReasoning';
import './reasoning-slider.css';

export function CodexReasoningSlider({ modelName, efforts, value, defaultEffort, choose, showModels, disabled = false }: {
  modelName: string; efforts: string[]; value: string; defaultEffort?: string;
  choose: (value: string) => void; showModels: () => void; disabled?: boolean;
}) {
  const effective = value || defaultEffort || '';
  const levels = visibleCodexEfforts(efforts, effective);
  if (!levels.length) return null;
  const index = levels.indexOf(effective);
  const known = index >= 0;
  const fraction = known ? levels.length === 1 ? 1 : index / (levels.length - 1) : 0;
  const label = known ? effortLabels[effective] ?? effective : '默认';
  const ultra = known && effective === 'ultra';
  const style = { '--reasoning-position': `${fraction * 100}%`, '--reasoning-thumb-offset': `${fraction * 26}px` } as CSSProperties;
  function selectStep(raw: string) {
    const level = levels[Number(raw)];
    if (level && !disabled) choose(level);
  }
  return <section className={`reasoning-slider-card${ultra ? ' is-ultra' : ''}`} aria-label="GPT 推理强度" style={style}>
    <div className="reasoning-slider-heading">
      <Zap size={14} className="reasoning-slider-bolt" fill="currentColor" aria-hidden="true"/>
      <strong aria-live="polite">{label}</strong>
      <button type="button" className="reasoning-slider-reset" title="恢复默认推理强度" aria-label="恢复默认推理强度" disabled={disabled || !value} onClick={() => choose('')}><RotateCcw size={14}/></button>
    </div>
    <button type="button" className="reasoning-slider-model" title={modelName} aria-label={`返回模型列表：${modelName}`} disabled={disabled} onClick={showModels}><span>{modelName}</span><ChevronRight size={11}/></button>
    <div className={`reasoning-slider-track${known ? '' : ' is-unset'}`}>
      <div className="reasoning-slider-fill" aria-hidden="true"/>
      <div className="reasoning-slider-stars" aria-hidden="true"/>
      <div className="reasoning-slider-ticks" aria-hidden="true">{levels.map((level, position) => <span key={level} style={{ left: `${levels.length === 1 ? 100 : position / (levels.length - 1) * 100}%` }} title={effortLabels[level] ?? level}/>)}</div>
      <input type="range" aria-label="推理强度" aria-valuetext={label} min={0} max={levels.length - 1} step={1} value={known ? index : 0} disabled={disabled}
        onInput={event => selectStep(event.currentTarget.value)}
        onPointerUp={event => { if (!known) selectStep(event.currentTarget.value); }}
        onKeyDown={event => {
          if (!known && ['Home', 'End', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) {
            event.preventDefault(); selectStep(String(event.key === 'End' ? levels.length - 1 : 0));
          }
        }}/>
    </div>
  </section>;
}
