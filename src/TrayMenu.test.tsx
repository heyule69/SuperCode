// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TrayMenu } from './TrayMenu';
import type { TraySnapshot } from './trayMenuModel';

const roots: ReturnType<typeof createRoot>[] = [];
beforeEach(() => { (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true; });
afterEach(() => { act(() => roots.splice(0).forEach(root => root.unmount())); document.body.innerHTML = ''; });
function mount(data: TraySnapshot) {
  const element = document.createElement('div'); document.body.append(element);
  const root = createRoot(element); roots.push(root);
  const perform = vi.fn(async () => {});
  act(() => root.render(createElement(TrayMenu, { data, error:'', perform, ready:() => {} })));
  return { element, perform };
}
function fixture(): TraySnapshot {
  return { token:1, hasMore:false, windows:[{label:'main',title:'会话 1',project:'项目',status:'idle',main:true}], recent:Array.from({length:8},(_,index)=>({id:String(index),title:`会话 ${index+1}`,project:'项目',status:index===0?'running':'idle',unread:true})) };
}
describe('tray menu', () => {
  it('opens the window list on pointer hover and keeps it available',()=>{
    const data=fixture();data.windows.push({label:'other',title:'另一个窗口',project:'test',status:'running',main:false});
    const{element}=mount(data);const button=element.querySelector('.tray-window-toggle')!;
    act(()=>button.dispatchEvent(new MouseEvent('pointerover',{bubbles:true})));
    expect(element.querySelector('.tray-window-list')?.textContent).toContain('另一个窗口');
    act(()=>element.querySelector('.tray-window-list')!.dispatchEvent(new MouseEvent('pointerover',{bubbles:true})));
    expect(element.querySelector('.tray-window-list')).not.toBeNull();
  });
  it('bounds the initial list, expands it and opens a chat with one click', async () => {
    const { element, perform } = mount(fixture());
    expect(element.querySelectorAll('.tray-chat')).toHaveLength(3);
    act(() => (element.querySelector('.tray-more') as HTMLButtonElement).click());
    expect(element.querySelectorAll('.tray-chat')).toHaveLength(8);
    await act(async () => (element.querySelector('.tray-chat') as HTMLButtonElement).click());
    expect(perform).toHaveBeenCalledWith('session', '0');
    expect(element.querySelector('.tray-status.running svg')).not.toBeNull();
    expect(element.querySelector('.tray-status.running .tray-unread')).toBeNull();
  });
  it('supports arrow-key focus and Escape dismissal', async () => {
    const { element, perform } = mount(fixture());
    const panel = element.querySelector('main')!;
    act(() => panel.dispatchEvent(new KeyboardEvent('keydown', {key:'ArrowDown',bubbles:true})));
    expect(document.activeElement?.textContent).toContain('会话 1');
    act(() => panel.dispatchEvent(new KeyboardEvent('keydown', {key:'End',bubbles:true})));
    expect(document.activeElement?.textContent).toBe('退出');
    await act(async () => panel.dispatchEvent(new KeyboardEvent('keydown', {key:'Escape',bubbles:true})));
    expect(perform).toHaveBeenCalledWith('dismiss', undefined);
  });
  it('exposes each workspace independently and keeps titles as text', async () => {
    const data = fixture(); data.windows.push({label:'chat-new',title:'<img src=x>',project:'另一个项目',status:'waiting',main:false});
    const { element, perform } = mount(data);
    act(() => (element.querySelector('.tray-window-toggle') as HTMLButtonElement).click());
    expect(element.querySelector('img')).toBeNull();
    const buttons = element.querySelectorAll<HTMLButtonElement>('.tray-window-list button');
    await act(async () => buttons[1].click());
    expect(perform).toHaveBeenCalledWith('window', 'chat-new');
    expect(buttons[1].querySelector('.tray-status.waiting')).not.toBeNull();
  });
});
