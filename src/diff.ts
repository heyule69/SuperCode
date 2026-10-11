function gitPath(value: string) {
  if (!value.startsWith('"')) return value;
  try {
    // Parse C escapes in order so an escaped backslash is never an octal byte.
    let result = '', bytes: number[] = [];
    const flush = () => { if (bytes.length) { result += new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array(bytes)); bytes = []; } };
    const escapes: Record<string, string> = { '\\': '\\', '"': '"', n: '\n', r: '\r', t: '\t', b: '\b', f: '\f', v: '\v', a: '\x07' };
    for (let index = 1; index < value.length - 1; index++) {
      if (value[index] === '\\') {
        const octal = /^[0-7]{3}/.exec(value.slice(index + 1));
        if (octal) { bytes.push(parseInt(octal[0], 8)); index += 3; continue; }
        flush(); result += escapes[value[++index]] ?? value[index];
      } else { flush(); result += value[index]; }
    }
    flush(); return result;
  } catch { return value.slice(1, -1); }
}
export function diffFiles(diff: string) {
  const files: { path: string; added: number; removed: number; diff: string }[] = [];
  let current: typeof files[number] | undefined;
  for (const line of diff.split('\n')) {
    if (line.startsWith('diff --git ')) {
      const quoted = / "b\/(.*)"$/.exec(line);
      const plain = / b\/(.*)$/.exec(line);
      const path = quoted ? gitPath(`"${quoted[1]}"`) : plain?.[1] ?? line;
      current = { path, added: 0, removed: 0, diff: `${line}\n` }; files.push(current);
    } else if (current) {
      current.diff += `${line}\n`;
      if (line.startsWith('+') && !line.startsWith('+++')) current.added++;
      if (line.startsWith('-') && !line.startsWith('---')) current.removed++;
    }
  }
  return files;
}
export function diffLines(text: string, limit = 5000) {
  let oldLine = 0, newLine = 0, hunk = false;
  return text.split('\n').slice(0, limit).map(text => {
    const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (match) { oldLine = Number(match[1]); newLine = Number(match[2]); hunk = true; return { text, kind: 'hunk', old: null, next: null }; }
    if (text.startsWith('diff --git')) hunk = false;
    if (!hunk || text.startsWith('\\')) return { text, kind: 'neutral', old: null, next: null };
    if (text.startsWith('+')) return { text, kind: 'added', old: null, next: newLine++ };
    if (text.startsWith('-')) return { text, kind: 'removed', old: oldLine++, next: null };
    if (text.startsWith(' ')) return { text, kind: 'neutral', old: oldLine++, next: newLine++ };
    return { text, kind: 'neutral', old: null, next: null };
  });
}
