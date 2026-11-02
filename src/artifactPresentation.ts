import { mediaKind } from './mediaSources';
import type { ArtifactFile } from './artifacts';

export function artifactSize(size:number):string {
  if(size<1024)return `${size} B`;
  const units=['KB','MB','GB','TB'];
  const index=Math.min(units.length-1,Math.floor(Math.log(size)/Math.log(1024))-1);
  return `${(size/1024**(index+1)).toFixed(1)} ${units[index]}`;
}

export function artifactFileLabel(file:ArtifactFile):string {
  if(file.kind==='directory')return '文件夹';
  const extension=file.name.includes('.')?file.name.split('.').at(-1)?.toLowerCase()??'':'';
  if(/^(?:htm|html)$/.test(extension))return '网页文件';
  if(/^(?:zip|7z|rar|tar|gz|bz2|xz)$/.test(extension))return '压缩包';
  if(/^(?:exe|msi|msix|appx|dmg|apk)$/.test(extension))return '程序文件';
  if(/^(?:xls|xlsx|csv|ods)$/.test(extension))return '表格';
  if(/^(?:ppt|pptx|odp)$/.test(extension))return '演示文稿';
  if(/^(?:doc|docx|pdf|odt|rtf|md|txt)$/.test(extension))return '文档';
  const media=mediaKind(file.path);
  if(media)return {image:'图片',video:'视频',audio:'音频'}[media];
  if(/^(?:[cm]?[jt]sx?|py|rs|go|java|c|cpp|h|cs|rb|sh|ps1|sql|toml|ya?ml|xml|ini|conf|json|css|scss)$/.test(extension))return '代码文件';
  return '文件';
}
