import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const decoder = new TextDecoder('utf-8', { fatal: true });
function read(path) {
  const bytes = readFileSync(new URL(`../${path}`, import.meta.url));
  assert.ok(!bytes.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf])), `${path}: UTF-8 BOM`);
  return decoder.decode(bytes);
}
const context = JSON.parse(read('resources/model-contexts.json'));
assert.match(context.verifiedAt, /^\d{4}-\d{2}-\d{2}$/);
for (const url of Object.values(context.sources)) assert.equal(new URL(url).protocol, 'https:');
const validWindow = value => Number.isSafeInteger(value) && value > 0 && value <= 100_000_000;
for (const [id, model] of Object.entries(context.models)) {
  assert.ok(validWindow(model.contextWindow), `Invalid window for ${id}`);
  assert.ok(context.sources[model.source], `Missing source for ${id}`);
  if (model.longContextWindow != null) assert.ok(validWindow(model.longContextWindow) && model.longContextWindow >= model.contextWindow);
}
for (const endpoint of context.endpointLimits) {
  assert.ok(context.sources[endpoint.source]);
  for (const host of endpoint.hosts) assert.equal(new URL(`https://${host}`).host, host);
  for (const [id, window] of Object.entries(endpoint.models)) {
    assert.ok(validWindow(window), `Invalid route model window ${id}`);
    if (context.models[id]) assert.ok(window <= (context.models[id].longContextWindow ?? context.models[id].contextWindow), `Route limit enlarges ${id}`);
  }
}
const codex = JSON.parse(read('resources/codex-api-models.json'));
assert.ok(codex.source.includes(`rust-v${codex.version}/`), 'Codex metadata source/version mismatch');
assert.ok(validWindow(codex.fallback.context_window));
const compatibility = JSON.parse(read('resources/agent-compatibility.json'));
assert.equal(compatibility.liveAgentValidation, false);
for (const entry of compatibility.agents) {
  for (const path of entry.fixtureTests) {
    const parts = path.split('::');
    assert.equal(parts.at(-2), 'tests', `Invalid compatibility fixture ${path}`);
    const source = read(`src-tauri/src/${parts.slice(0, -2).join('/')}.rs`);
    assert.match(source, new RegExp(`\\bfn\\s+${parts.at(-1)}\\s*\\(`), `Missing compatibility fixture ${path}`);
  }
}
console.log(`Validated ${Object.keys(context.models).length} model windows, endpoint limits, Codex metadata, and ${compatibility.agents.length} agent compatibility records.`);
