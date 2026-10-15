// A connection-test-only observer. Keep the CLI's endpoint, headers and payload
// unchanged, and report the first model-request failure before SDK backoff.
import { writeSync } from 'node:fs';
const fetchOriginal = globalThis.fetch;
const secret = process.env.GEMINI_API_KEY || '';
const mask = text => secret ? String(text).replaceAll(secret, '[REDACTED]') : String(text);
const compact = text => [...mask(text)].filter(c => c >= ' ' && c !== '\u007f').slice(0, 512).join('');
let reported = false;
function publish(details) {
  if (reported) return;
  reported = true;
  // ACP redirects process.stdout.write into its output event bus. Its original
  // pipe is still fd 1; diagnostics must reach the host's JSONL reader directly.
  try { writeSync(1, JSON.stringify({ type: 'supercode_gemini_probe_failure', ...details }) + '\n'); }
  catch { /* The test owner may already have closed its pipe. */ }
}
function endpoint(input) {
  try {
    const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url);
    const base = new URL(process.env.SUPERCODE_GEMINI_PROBE_BASE);
    const prefix = base.pathname.replace(/\/$/, '') + '/';
    if (url.origin !== base.origin || !url.pathname.startsWith(prefix)
      || !/:(?:streamGenerateContent|generateContent)$/.test(url.pathname)) return null;
    return compact(url.origin + url.pathname);
  } catch { return null; }
}
async function errorReason(response) {
  let reader, timer;
  try {
    reader = response.clone().body?.getReader();
    if (!reader) return '';
    const bytes = await Promise.race([
      (async () => {
        let length = 0;
        const chunks = [];
        while (true) {
          const chunk = await reader.read();
          if (chunk.done) break;
          length += chunk.value.length;
          if (length > 32 * 1024) return null;
          chunks.push(Buffer.from(chunk.value));
        }
        return Buffer.concat(chunks);
      })(),
      new Promise(resolve => { timer = setTimeout(() => resolve(null), 1000); }),
    ]);
    if (!bytes) return '';
    const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    const reason = [value.error?.message, value.error, value.message, value.detail]
      .find(value => typeof value === 'string' && value.trim());
    return reason ? compact(reason) : '';
  } catch { return ''; }
  finally {
    clearTimeout(timer);
    // Cancellation of a cloned tee can wait for the CLI's other branch. Never
    // wait for that branch before returning its original response.
    if (reader) void reader.cancel().catch(() => {});
  }
}
globalThis.fetch = async function(input, options) {
  const target = endpoint(input);
  let response;
  try { response = await Reflect.apply(fetchOriginal, globalThis, [input, options]); }
  catch (error) {
    if (target) publish({ endpoint: target, transportError: true });
    throw error;
  }
  if (target && response.status >= 400 && !reported) {
    publish({ endpoint: target, status: response.status, reason: await errorReason(response) });
  }
  return response;
};
