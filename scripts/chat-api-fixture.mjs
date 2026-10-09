// Explicit protocol fixture for SuperCode + the real Claude CLI; no external model API.
import { createServer } from 'node:http';
import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
mkdirSync('.supercode', { recursive: true });
let calls = 0; let toolResults = 0;
const server = createServer(async (req, res) => {
  if (req.headers.authorization !== 'Bearer fixture-key') { res.writeHead(401).end(); return; }
  if (req.url === '/v1/models' && req.method === 'GET') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ data: [{ id: 'supercode-fixture-model' }] })); return; }
  if (req.url !== '/v1/chat/completions') { res.writeHead(404).end(); return; }
  const buffers = []; let bytes = 0;
  for await (const chunk of req) { bytes += chunk.length; if (bytes > 16 * 1024 * 1024) { res.writeHead(413).end(); return; } buffers.push(chunk); }
  let body; try { body = JSON.parse(Buffer.concat(buffers).toString('utf8')); } catch { res.writeHead(400).end(); return; }
  if (body.model !== 'supercode-fixture-model') { res.writeHead(400).end(); return; }
  calls++;
  const messages = body.messages ?? [];
  const prompt = JSON.stringify([...messages].reverse().find(m => m.role === 'user')?.content ?? '');
  const token = JSON.stringify(messages).match(/SC-[a-f0-9]{10}/)?.[0] ?? 'OK';
  const read = prompt.includes('读取当前项目');
  const hasResult = messages.some(m => m.role === 'tool' && String(m.content).includes('SuperCode'));
  if (hasResult) toolResults++;
  writeFileSync('.supercode/chat-fixture-observed.json', JSON.stringify({ fixture: true, calls, toolResults, model: body.model }, null, 2), 'utf8');
  if (!body.stream) { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ id: 'fixture', model: body.model, choices: [{ message: { content: 'OK' }, finish_reason: 'stop' }] })); return; }
  res.setHeader('Content-Type', 'text/event-stream');
  const delta = (value, finish = null) => res.write(`data: ${JSON.stringify({ choices: [{ delta: value, finish_reason: finish }] })}\n\n`);
  delta({ reasoning_content: '协议测试：准备返回文本或请求读取 README。' });
  if (prompt.includes('10000')) {
    delta({ content: '1\n' }); const timer = setTimeout(() => { delta({}, 'stop'); res.end('data: [DONE]\n\n'); }, 60000);
    res.on('close', () => clearTimeout(timer));
  } else if (read && !hasResult) {
    delta({ tool_calls: [{ index: 0, id: `fixture-read-${calls}`, type: 'function', function: { name: 'Read', arguments: JSON.stringify({ file_path: resolve('README.md') }) } }] }, 'tool_calls'); res.end('data: [DONE]\n\n');
  } else { delta({ content: read && hasResult ? '# SuperCode' : token }, 'stop'); res.end('data: [DONE]\n\n'); }
});
server.listen(0, '127.0.0.1', () => {
  const baseUrl = `http://127.0.0.1:${server.address().port}/v1`;
  writeFileSync('.supercode/chat-fixture.json', JSON.stringify({ baseUrl }, null, 2), 'utf8'); console.log(`Protocol fixture ready: ${baseUrl}`);
});
