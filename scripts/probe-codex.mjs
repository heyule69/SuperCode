// Read-only protocol diagnostic. No inference request and no credential output.
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { delimiter, join } from 'node:path';
let executable = process.env.SUPERCODE_CODEX_PATH;
for (const directory of executable ? [] : (process.env.PATH ?? '').split(delimiter)) {
  const native = join(directory, 'node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe');
  const ordinary = join(directory, process.platform === 'win32' ? 'codex.exe' : 'codex');
  if (existsSync(native)) { executable = native; break; }
  if (existsSync(ordinary)) { executable = ordinary; break; }
}
if (!executable) throw new Error('未找到原生 Codex CLI');
const child = spawn(executable, ['app-server', '--listen', 'stdio://', '-c', 'service_tier="fast"'], { stdio: ['pipe', 'pipe', 'ignore'], windowsHide: true });
const timer = setTimeout(() => { child.kill(); process.exitCode = 1; }, 20000);
let buffer = '';
const send = value => child.stdin.write(JSON.stringify(value) + '\n');
child.stdout.setEncoding('utf8');
child.stdout.on('data', chunk => {
  buffer += chunk;
  for (;;) {
    const end = buffer.indexOf('\n'); if (end < 0) break;
    const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
    const message = JSON.parse(line);
    if (message.id === 1) { send({ method: 'initialized', params: {} }); send({ id: 2, method: 'model/list', params: { limit: 100 } }); }
    if (message.id === 2) {
      console.log(JSON.stringify(message.result?.data?.map(m => ({ model: m.model, name: m.displayName, isDefault: m.isDefault })) ?? message.error, null, 2));
      clearTimeout(timer); child.kill();
    }
  }
});
send({ id: 1, method: 'initialize', params: { clientInfo: { name: 'supercode_diagnostic', title: 'SuperCode diagnostic', version: '0.1.0' } } });
