// @vitest-environment happy-dom
import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Sidebar } from './Sidebar';
import { sidebarTime } from './sidebarTime';
import type { Session } from './types';
vi.mock('./api', () => ({ desktop:false,call:vi.fn() }));
let root: Root, host: HTMLDivElement, props: ComponentProps<typeof Sidebar>;
const session = (id: string, projectId: string): Session => ({id,projectId,title:id,agent:'claude',model:null,nativeId:null,status:'idle',updatedAt:Date.now()/1000-3600,turnId:null});
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; localStorage.clear();
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  props = { projects:[{id:'p',name:'项目 A',path:'D:/A'}],sessions:[session('独立任务',''),session('项目会话','p')],state:{projects:{},sessions:{},sections:[]},projectId:'p',sessionId:'独立任务',ready:true,blocked:false,collapsed:new Set(),setCollapsed:vi.fn(),selectProject:vi.fn(),selectSession:vi.fn(),addProject:vi.fn(),createSession:vi.fn(async()=>{}),updated:vi.fn(async()=>{}),dialogChanged:vi.fn() };
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
it('separates standalone tasks from projects and preserves independent section collapse and new-task creation', async () => {
  await act(async () => root.render(<Sidebar {...props}/>));
  const tasks = host.querySelector('[aria-label="任务列表"]')!;
  const projects = host.querySelector('[aria-label="项目列表"]')!;
  expect(tasks.textContent).toContain('独立任务'); expect(tasks.textContent).not.toContain('项目会话');
  expect(projects.textContent).toContain('项目会话'); expect(projects.textContent).not.toContain('独立任务');
  expect(host.querySelector('time')?.textContent).toBe('1小时前');
  await act(async () => tasks.querySelector<HTMLButtonElement>('.sidebar-section-toggle')!.click());
  expect(tasks.querySelector('.smooth-collapse')?.hasAttribute('inert')).toBe(true);
  expect(projects.querySelector('.smooth-collapse')?.getAttribute('aria-hidden')).toBe('false');
  await act(async () => root.unmount()); root = createRoot(host);
  await act(async () => root.render(<Sidebar {...props}/>));
  expect(host.querySelector('.sidebar-direct .sidebar-section-toggle')?.getAttribute('aria-expanded')).toBe('false');
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="新建任务"]')!.click());
  expect(props.createSession).toHaveBeenCalledWith();
});
it('formats real update times and handles future or missing timestamps', () => {
  const now = Date.UTC(2026,9,11,12);
  expect(sidebarTime(now/1000-180,now)).toBe('3分钟前');
  expect(sidebarTime(now/1000-86400*2,now)).toBe('2天前');
  expect(sidebarTime(now/1000+60,now)).toBe('刚刚');
  expect(sidebarTime(NaN,now)).toBe('');
});
