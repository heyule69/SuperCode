// @vitest-environment happy-dom
import { act, type ComponentProps } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Sidebar } from './Sidebar';
import { call } from './api';
import { sidebarTime } from './sidebarTime';
import type { Session } from './types';
const platform = vi.hoisted(() => ({ desktop:false }));
vi.mock('./api', () => ({ get desktop() { return platform.desktop; }, call:vi.fn() }));
let root: Root, host: HTMLDivElement, props: ComponentProps<typeof Sidebar>;
const session = (id: string, projectId: string): Session => ({id,projectId,title:id,agent:'claude',model:null,nativeId:null,status:'idle',updatedAt:Date.now()/1000-3600,turnId:null});
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true; localStorage.clear(); vi.mocked(call).mockReset();
  platform.desktop = false;
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  props = { projects:[{id:'p',name:'项目 A',path:'D:/A'}],sessions:[session('独立任务',''),session('项目会话','p')],state:{projects:{},sessions:{},sections:[]},projectId:'p',sessionId:'独立任务',ready:true,blocked:false,collapsed:new Set(),setCollapsed:vi.fn(),selectProject:vi.fn(),selectSession:vi.fn(),addProject:vi.fn(),createSession:vi.fn(async()=>{}),updated:vi.fn(async()=>{}),dialogChanged:vi.fn() };
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
function menuButton(label: string) { return [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button => button.textContent === label)!; }

it('dismisses project launch menus before IPC finishes, keeps the sidebar interactive, and skips refreshing unchanged data', async () => {
  platform.desktop = true;
  await act(async () => root.render(<Sidebar {...props}/>));
  const origin = host.querySelector<HTMLButtonElement>('[aria-label="项目 A 的项目菜单"]')!;
  for (const [label, mode] of [['在资源管理器中打开','folder'], ['VS Code','vscode'], ['系统终端','terminal']]) {
    let finish!: () => void;
    vi.mocked(call).mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
    await act(async () => origin.click());
    if (mode !== 'folder') await act(async () => menuButton('打开方式').click());
    await act(async () => menuButton(label).click());
    expect(call).toHaveBeenLastCalledWith('open_sidebar_project', { projectId:'p', mode });
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.activeElement).toBe(origin);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="置顶 项目会话"]')?.disabled).toBe(false);
    expect(props.updated).not.toHaveBeenCalled();
    await act(async () => origin.click());
    const nextMenu = document.querySelector('[role="menu"][aria-label="项目操作"]');
    expect(nextMenu).not.toBeNull();
    expect(menuButton('在资源管理器中打开').disabled).toBe(false);
    await act(async () => finish());
    expect(document.querySelector('[role="menu"][aria-label="项目操作"]')).toBe(nextMenu);
    expect(props.updated).not.toHaveBeenCalled();
    await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key:'Escape', bubbles:true })));
  }
});

it('reports a delayed launch failure outside the menu without closing a newly opened menu', async () => {
  platform.desktop = true;
  let fail!: (error: Error) => void;
  vi.mocked(call).mockImplementationOnce(() => new Promise<void>((_, reject) => { fail = reject; }));
  await act(async () => root.render(<Sidebar {...props}/>));
  const origin = host.querySelector<HTMLButtonElement>('[aria-label="项目 A 的项目菜单"]')!;
  await act(async () => origin.click());
  await act(async () => menuButton('在资源管理器中打开').click());
  expect(document.querySelector('[role="menu"]')).toBeNull();
  await act(async () => origin.click());
  const nextMenu = document.querySelector('[role="menu"]');
  await act(async () => fail(new Error('项目文件夹不存在')));
  expect(document.querySelector('[role="menu"]')).toBe(nextMenu);
  expect(nextMenu?.querySelector('[role="alert"]')).toBeNull();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('项目文件夹不存在');
  await act(async () => host.querySelector<HTMLButtonElement>('[title="关闭打开失败提示"]')!.click());
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it('separates standalone tasks from projects and preserves independent section collapse and new-task creation', async () => {
  await act(async () => root.render(<Sidebar {...props}/>));
  const tasks = host.querySelector('[aria-label="任务列表"]')!;
  const projects = host.querySelector('[aria-label="项目列表"]')!;
  expect(tasks.textContent).toContain('独立任务'); expect(tasks.textContent).not.toContain('项目会话');
  expect(projects.textContent).toContain('项目会话'); expect(projects.textContent).not.toContain('独立任务');
  expect(host.querySelector('.sidebar-chat-row > .sidebar-row-menu')).toBeNull();
  await act(async () => tasks.querySelector<HTMLButtonElement>('.sidebar-section-toggle')!.click());
  expect(tasks.querySelector('.smooth-collapse')?.hasAttribute('inert')).toBe(true);
  expect(projects.querySelector('.smooth-collapse')?.getAttribute('aria-hidden')).toBe('false');
  await act(async () => root.unmount()); root = createRoot(host);
  await act(async () => root.render(<Sidebar {...props}/>));
  expect(host.querySelector('.sidebar-direct .sidebar-section-toggle')?.getAttribute('aria-expanded')).toBe('false');
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="新建任务"]')!.click());
  expect(props.createSession).toHaveBeenCalledWith();
});
it('pins and archives a background chat directly without selecting it or changing project controls', async () => {
  await act(async () => root.render(<Sidebar {...props}/>));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="置顶 项目会话"]')!.click());
  expect(call).toHaveBeenCalledWith('update_sidebar_item', { kind:'session', id:'项目会话', action:'pin', value:true });
  expect(props.updated).toHaveBeenCalledOnce(); expect(props.selectSession).not.toHaveBeenCalled();
  props = { ...props, state:{ ...props.state, sessions:{ '项目会话':{ pinned:true, unread:false, sectionId:null } } } };
  await act(async () => root.render(<Sidebar {...props}/>));
  expect(host.querySelector('[aria-label="取消置顶 项目会话"]')?.getAttribute('aria-pressed')).toBe('true');
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="取消置顶 项目会话"]')!.click());
  expect(call).toHaveBeenCalledWith('update_sidebar_item', { kind:'session', id:'项目会话', action:'pin', value:false });
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="归档 项目会话"]')!.click());
  expect(call).toHaveBeenCalledWith('archive_session', { sessionId:'项目会话' });
  expect(props.selectSession).not.toHaveBeenCalled();
  const projectMenu = host.querySelector<HTMLButtonElement>('[aria-label="项目 A 的项目菜单"]')!;
  expect(projectMenu).not.toBeNull();
  await act(async () => projectMenu.click());
  expect(document.querySelector('[role="menu"][aria-label="项目操作"]')?.textContent).toContain('编辑项目');
});
it('opens chat actions by right click or keyboard, restores focus, and keeps active chats from being archived', async () => {
  props.sessions[1] = { ...props.sessions[1], status:'waiting', turnId:'active' };
  await act(async () => root.render(<Sidebar {...props}/>));
  const waitingChat = host.querySelector<HTMLButtonElement>('.session-row[title="项目会话"]')!;
  expect(waitingChat.querySelector('.session-input-badge')?.textContent).toBe('需要用户输入');
  expect(waitingChat.querySelector('.waiting-badge')).toBeNull();
  expect(waitingChat.querySelector('[aria-label="等待确认"]')).not.toBeNull();
  props.sessions[1] = { ...props.sessions[1], status:'running', turnId:'active' };
  await act(async () => root.render(<Sidebar {...props}/>));
  const chat = host.querySelector<HTMLButtonElement>('.session-row[title="项目会话"]')!;
  expect(chat.querySelector('.session-input-badge')).toBeNull();
  expect(chat.querySelector('[aria-label="运行中"]')).not.toBeNull();
  expect(host.querySelector<HTMLButtonElement>('[aria-label="归档 项目会话"]')?.disabled).toBe(true);
  await act(async () => chat.dispatchEvent(new MouseEvent('contextmenu', { bubbles:true, cancelable:true, clientX:40, clientY:90 })));
  expect(document.querySelector('[role="menu"][aria-label="聊天操作"]')).not.toBeNull();
  expect(props.selectSession).not.toHaveBeenCalled();
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key:'Escape', bubbles:true })));
  expect(document.querySelector('[role="menu"][aria-label="聊天操作"]')).toBeNull();
  expect(document.activeElement).toBe(chat);
  await act(async () => chat.dispatchEvent(new KeyboardEvent('keydown', { key:'F10', shiftKey:true, bubbles:true, cancelable:true })));
  const menu = document.querySelector('[role="menu"][aria-label="聊天操作"]')!;
  expect(menu).not.toBeNull();
  const archive = [...menu.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.startsWith('归档'))!;
  expect(archive.disabled).toBe(true);
  await act(async () => archive.click());
  expect(call).not.toHaveBeenCalledWith('archive_session', expect.anything());
});
it('formats real update times and handles future or missing timestamps', () => {
  const now = Date.UTC(2026,9,11,12);
  expect(sidebarTime(now/1000-180,now)).toBe('3分钟前');
  expect(sidebarTime(now/1000-86400*2,now)).toBe('2天前');
  expect(sidebarTime(now/1000+60,now)).toBe('刚刚');
  expect(sidebarTime(NaN,now)).toBe('');
});
