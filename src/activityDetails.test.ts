import { expect, it } from 'vitest';
import { activityCommand, activityDiff, activityFiles, activityOutput } from './activityDetails';
import { diffFiles } from './diff';
import type { Message } from './types';
const message=(kind:string,text:string,data:Message['data']):Message=>({id:'operation',seq:1,sessionId:'s',role:'tool',kind,text,data});
it('uses exact per-operation counts rather than counting a truncated code preview',()=>{
  const row=message('claudeToolCall','saved',{tool:'Edit',changeSummary:[{path:'中文.ts',kind:{type:'update'},added:4000,removed:1}],changes:[{path:'中文.ts',diff:'@@ -1 +1 @@\n-old\n+new\n… 输出过长，已截断'}]});
  expect(activityFiles(row)[0]).toMatchObject({path:'中文.ts',added:4000,removed:1,truncated:true});
  expect(activityFiles({...row,data:{changes:[{path:'中文.ts',diff:'@@ -1 +1 @@\n---old-code\n+++new-code\n'}]}})[0]).toMatchObject({added:1,removed:1});
  expect(activityFiles({...row,data:{changes:[{path:'中文.ts',diff:'@@ -1 +1 @@\n-old\n+new\n… 输出过长，已截断'}]}})[0].added).toBeUndefined();
});
it('preserves full commands and output with and without the historical command prefix',()=>{
  const command='node "中文/check.mjs" && echo $(literal)';
  const cli=message('commandExecution',command+'\nFIRST_OUTPUT\nSECOND_OUTPUT',{command});
  expect(activityCommand(cli)).toBe(command);
  expect(activityOutput(cli)).toBe('FIRST_OUTPUT\nSECOND_OUTPUT');
  expect(activityOutput({...cli,text:'RAW_OUTPUT'})).toBe('RAW_OUTPUT');
  expect(activityCommand(message('claudeToolCall','BASH_OUTPUT',{tool:'Bash',arguments:{command}}))).toBe(command);
  expect(activityOutput(message('fileChange','path.ts',{operationOutput:'PATCH_OUTPUT'}))).toBe('PATCH_OUTPUT');
});
it('preserves operation patches and supplies file identities for headerless native hunks in the side review',()=>{
  const original='diff --git a/中文.ts b/中文.ts\n@@ -1 +1 @@\n-old\n+new\n';
  const text=activityDiff([{path:'中文.ts',diff:original},{path:'next.ts',diff:'@@ -1 +1 @@\n-a\n+b\n'},{path:'no-patch.ts'}]);
  expect(text.startsWith(original)).toBe(true);
  expect(diffFiles(text).map(file=>file.path)).toEqual(['中文.ts','next.ts']);
  expect(activityDiff([])).toBe('');
});
