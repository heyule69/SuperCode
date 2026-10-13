// Controlled JSONL fixture. It verifies launch metadata without running an agent.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const readline = require('node:readline');
const expected = JSON.parse(process.argv[2]);
const config = Object.fromEntries(expected.overrides.map(value => {
  const equal = value.indexOf('=');
  return [value.slice(0, equal), value.slice(equal + 1)];
}));
assert.equal(config.model_auto_compact_token_limit, undefined);
const encodedPath = config.model_catalog_json;
const path = encodedPath.startsWith("'") ? encodedPath.slice(1, -1) : JSON.parse(encodedPath);
const bytes = fs.readFileSync(path);
assert.notDeepEqual([...bytes.subarray(0, 3)], [239, 187, 191]);
const rows = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)).models;
const ceiling = Number(config.model_context_window);
for (const [model, limits] of Object.entries(expected.models)) {
  const row = rows.find(row => row.slug === model);
  assert.ok(row, `Missing model ${model}`);
  assert.equal(row.context_window, limits.window);
  assert.equal(row.max_context_window, limits.window);
  assert.equal(row.effective_context_window_percent, limits.percent);
  assert.equal(row.auto_compact_token_limit, limits.auto);
  assert.ok(row.model_messages.instructions_template.length > 1000);
  assert.ok(ceiling >= limits.window);
}
assert.ok(ceiling >= 272000);
assert.ok(rows.every(row => row.context_window <= ceiling));
assert.equal(rows.some(row => row.slug === 'unknown'), false);
let initialized = false, resumed = false;
readline.createInterface({ input: process.stdin }).on('line', line => {
  const request = JSON.parse(line);
  let result;
  if (request.method === 'initialize') {
    initialized = true; result = { protocolVersion: 'fixture' };
  } else if (request.method === 'thread/resume') {
    assert.ok(initialized);
    assert.equal(request.params.threadId, expected.native);
    assert.equal(request.params.model, expected.model);
    assert.equal(request.params.developerInstructions, undefined);
    resumed = true; result = { thread: { id: expected.native } };
  } else if (request.method === 'turn/start') {
    assert.ok(resumed);
    assert.equal(request.params.threadId, expected.native);
    assert.equal(request.params.model, expected.model);
    result = { turn: { id: 'fixture-turn' } };
  } else {
    assert.fail(`Unexpected method ${request.method}`);
  }
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\n');
});
