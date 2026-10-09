import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const inventory = spawnSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], { encoding: 'utf8' });
if (inventory.status !== 0) throw new Error('无法读取项目文件列表');
const decoder = new TextDecoder('utf-8', { fatal: true });
let count = 0;
for (const path of inventory.stdout.split('\0').filter(Boolean)) {
  if (/\.(png|ico|icns|jpg|jpeg|webp|gif|exe)$/i.test(path)) continue;
  const bytes = readFileSync(path);
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) throw new Error(`文件含 UTF-8 BOM：${path}`);
  try { decoder.decode(bytes); } catch { throw new Error(`文件不是有效 UTF-8：${path}`); }
  count++;
}
console.log(`已验证 ${count} 个文本文件：UTF-8，无 BOM。`);
