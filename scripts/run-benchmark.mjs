import { mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
mkdirSync('.supercode/benchmarks', { recursive: true });
const result = spawnSync(process.execPath, ['node_modules/vitest/vitest.mjs', 'bench', '--run', '--outputJson', '.supercode/benchmarks/conversation.json'], { stdio: 'inherit', windowsHide: true, shell: false });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
