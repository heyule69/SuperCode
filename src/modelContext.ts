import limits from '../resources/model-contexts.json';
import official from '../resources/official-models.json';
import type { Model } from './types';

const positive = (value?: number | null) => typeof value === 'number' && Number.isSafeInteger(value) && value > 0 && value <= 100_000_000 ? value : undefined;
export function modelContextWindow(model: string, metadata?: Model, officialCodex = false): number | undefined {
  const declared = positive(metadata?.contextWindow);
  if (declared) {
    const percent = positive(metadata?.effectiveContextWindowPercent);
    return percent && percent <= 100 ? Math.floor(declared * percent / 100) : declared;
  }
  // Codex's account catalog reports its harness window, which differs from
  // the same model's API specification. Never apply these limits to API routes.
  if (officialCodex) {
    const preset = official.models.find(entry => entry.model === model);
    if (preset) return Math.floor(preset.contextWindow * preset.effectiveContextWindowPercent / 100);
    return undefined;
  }
  const id = model.toLowerCase().split('/').at(-1) ?? '';
  const variant = id.match(/\[(1m|256k)\]$/);
  const base = variant ? id.slice(0, -variant[0].length) : id;
  const spec = (limits.models as Record<string, { contextWindow: number; longContextWindow?: number }>)[base];
  if (!spec) return undefined;
  // A CLI annotation selects a verified variant; it cannot add capacity to a
  // model or endpoint that only supports a smaller window.
  return variant?.[1] === '1m' ? spec.longContextWindow ?? spec.contextWindow
    : variant?.[1] === '256k' ? Math.min(262_144, spec.contextWindow) : spec.contextWindow;
}

// Match Codex's compact tooltip while retaining exact counts for accessibility.
export function contextNumber(value: number) {
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(2))}m`;
  if (value >= 1000) return `${Math.round(value / 1000)}k`;
  return String(value);
}
