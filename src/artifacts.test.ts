import { expect,it } from 'vitest';
import { referencedArtifacts,turnArtifactPaths } from './artifacts';
import { isActivity,isDisplayEvent,mergeEvent } from './events';
import { presentTurn } from './turnPresentation';
import type { Message } from './types';

const message=(id:string,text:string,phase?:string):Message=>({id,seq:0,sessionId:'s',role:'assistant',kind:'agentMessage',text,data:{phase}});
it('recognizes final Windows and relative file references while excluding previews, URLs, code examples and line references',()=>{
  const text='[表格](<C:\\Users\\中文目录 with spaces\\报告.xlsx>)\n\n[压缩包](outputs/结果.7z)\n\n`outputs/unknown.custom`\n\n![预览](<C:\\work\\preview.png>)\n\n[网站](https://example.com)\n\n`src/App.tsx:12`\n\n```sh\n[example](fake.xlsx)\n```';
  expect(referencedArtifacts(text)).toEqual(['C:\\Users\\中文目录 with spaces\\报告.xlsx','outputs/结果.7z','outputs/unknown.custom']);
});
it('does not invent a product for normal chat, progress, a bare filename or an embedded preview',()=>{
  expect(referencedArtifacts('你好，暂时没有生成文件。')).toEqual([]);
  expect(referencedArtifacts('文件还在生成 report.xlsx\n\n![过程](work/preview.png)')).toEqual([]);
  expect(turnArtifactPaths([],[])).toEqual([]);
});
it('does not promote a mentioned configuration or signing-key location into a final download',()=>{
  expect(referencedArtifacts('私钥已保存到 `C:\\Users\\Administrator\\.supercode-signing\\update.key`，没有上传到 GitHub。\n\n修改了 `src/App.tsx`。')).toEqual([]);
});
it('retains discovered outputs across live events and saved history without showing them as tool activity',()=>{
  const event={method:'turn/artifacts/updated',params:{threadId:'native',turnId:'t',artifacts:[{path:'D:/outputs/报告.xlsx',name:'报告.xlsx',size:128}],error:null}};
  expect(isDisplayEvent(event)).toBe(true);
  const live=mergeEvent([],event,'s');expect(isActivity(live[0])).toBe(false);
  const final=message('final','已完成。','final_answer');
  expect(turnArtifactPaths(JSON.parse(JSON.stringify(live)),[final])).toEqual(['D:/outputs/报告.xlsx']);
  const presentation=presentTurn({id:'t',run:{...live[0],kind:'runMarker',data:{status:'completed',turnId:'t'}},items:[...live,final]});
  expect(presentation.process).toEqual([]);expect(presentation.final).toEqual([final]);
  const explicit=message('final','[报告](D:/outputs/报告.xlsx)');
  expect(turnArtifactPaths(live,[explicit])).toEqual(['D:/outputs/报告.xlsx']);
});
