import {expect,it} from 'vitest';
import {artifactFileLabel,artifactSize} from './artifactPresentation';
import {artifactPathKey,turnKnownArtifacts} from './artifacts';
import type {Message} from './types';

it.each([['report.xlsx','表格'],['slides.pptx','演示文稿'],['report.pdf','文档'],['index.HTML','网页文件'],['bundle.7z','压缩包'],['setup.exe','程序文件'],['unknown.custom','文件'],['无扩展名','文件'],['html','文件'],['exe','文件'],['main.rs','代码文件'],['demo.mp4','视频'],['picture.png','图片'],['voice.mp3','音频']])('labels %s without reading or restricting file contents',(name,label)=>{
  expect(artifactFileLabel({name,path:`D:/outputs/${name}`,size:0})).toBe(label);
});
it('formats empty and large files without mistaking a directory for a zero-byte file',()=>{
  expect(artifactSize(0)).toBe('0 B');
  expect(artifactSize(1024)).toBe('1.0 KB');
  expect(artifactSize(3*1024**3)).toBe('3.0 GB');
  expect(artifactFileLabel({kind:'directory',name:'project',path:'D:/outputs/project',size:0})).toBe('文件夹');
});
it('retains only real historical artifact metadata and reconciles Windows aliases',()=>{
  const file={path:'D:\\outputs\\报告.xlsx',name:'报告.xlsx',size:0};
  const items:Message[]=[{id:'files',seq:1,sessionId:'s',role:'activity',text:'',kind:'turnArtifacts',data:{artifacts:[file,{...file,path:'d:/outputs/报告.xlsx'},{path:'missing'},null]}}];
  expect(turnKnownArtifacts(items)).toHaveLength(1);
  expect(artifactPathKey('file:///D:/outputs/%E6%8A%A5%E5%91%8A.xlsx')).toBe(artifactPathKey(file.path));
});
