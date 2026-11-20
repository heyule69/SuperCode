import { expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { LinkageChecklist } from './LinkageChecklist';
import type { LinkageTask } from './useLinkage';

const task: LinkageTask={objective:'完成整份计划',state:'needs_review',round:1,steps:[{text:'第一步',done:true,evidence:'文件与测试已核对'},{text:'第二步',done:false,evidence:'尚未执行'},{text:'验证结果',done:false,evidence:'尚未验证'}]};
it('shows one verified step as pending review, with the remaining checklist intact',()=>{
  const html=renderToStaticMarkup(<LinkageChecklist task={task}/>);
  expect(html).toContain('待主控验收');expect(html).toContain('1 / 3');expect(html).toContain('第二步');expect(html).not.toContain('已验收');
});
it('keeps the real blocker, attempts and unfinished items visible',()=>{
  const html=renderToStaticMarkup(<LinkageChecklist task={{...task,state:'blocked',reason:'缺少测试账号',attempts:['已检查项目配置和现有登录方式']}}/>);
  expect(html).toContain('遇到阻碍');expect(html).toContain('缺少测试账号');expect(html).toContain('已检查项目配置');expect(html).toContain('1 / 3');
});
it('renders executor evidence as text rather than active markup',()=>{
  const html=renderToStaticMarkup(<LinkageChecklist task={{...task,steps:[{text:'<script>alert(1)</script>',done:false,evidence:'<img src=x onerror=alert(1)>'}]}}/>);
  expect(html).not.toContain('<script>');expect(html).not.toContain('<img');
});
