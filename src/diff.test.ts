import { expect, it } from 'vitest';
import { diffFiles, diffLines } from './diff';
it('counts changed content and tracks both sides across hunks', () => {
  const diff = 'diff --git a/中文 文件.ts b/中文 文件.ts\n--- a/中文 文件.ts\n+++ b/中文 文件.ts\n@@ -10,2 +10,3 @@\n same\n-old\n+new\n+extra\n@@ -30 +31 @@\n-last\n+final';
  expect(diffFiles(diff)[0]).toMatchObject({ path: '中文 文件.ts', added: 3, removed: 2 });
  const lines = diffLines(diff);
  expect(lines.find(l => l.text === '-old')).toMatchObject({ old: 11, next: null });
  expect(lines.find(l => l.text === '+extra')).toMatchObject({ old: null, next: 12 });
  expect(lines.find(l => l.text === '+final')).toMatchObject({ old: null, next: 31 });
});
it('separates files and handles quoted filenames without including diff headers', () => {
  const diff = 'diff --git "a/a b.ts" "b/a b.ts"\n--- "a/a b.ts"\n+++ "b/a b.ts"\n@@ -0,0 +1 @@\n+first\ndiff --git a/b.ts b/b.ts\n@@ -1 +1 @@\n-old\n+new';
  expect(diffFiles(diff).map(f => [f.path, f.added, f.removed])).toEqual([['a b.ts', 1, 0], ['b.ts', 1, 1]]);
});
it('decodes Git octal UTF-8 paths without interpreting escaped backslashes as octal bytes', () => {
  const diff = String.raw`diff --git "a/\344\270\255\346\226\207.ts" "b/\344\270\255\346\226\207.ts"
@@ -1 +1 @@
-old
+new
diff --git "a/\\345.txt" "b/\\345.txt"
@@ -1 +1 @@
-old
+new`;
  expect(diffFiles(diff).map(file => file.path)).toEqual(['中文.ts', String.raw`\345.txt`]);
});
