// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { NewChat } from './NewChat';
import type { Project } from './types';

const projects: Project[] = [
  { id:'one', name:'SuperCode', path:'D:/项目/SuperCode' },
  { id:'two', name:'另一个项目', path:'D:/另一个项目' },
];
let root: Root, container: HTMLDivElement;
const selectProject = vi.fn(), addProject = vi.fn();
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  selectProject.mockClear(); addProject.mockClear();
  container = document.createElement('div'); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
async function render(items = projects) {
  await act(async () => root.render(<NewChat project={items[0]} projects={items} selectProject={selectProject} addProject={addProject}/>));
}
const trigger = () => container.querySelector<HTMLButtonElement>('[aria-label="选择项目"]')!;
async function open() { await act(async () => trigger().click()); }

it('switches projects through the existing callback and leaves the current project untouched when reselected', async () => {
  await render(); await open();
  const options = () => [...container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
  expect(options()[0].getAttribute('aria-checked')).toBe('true');
  await act(async () => options()[0].click());
  expect(selectProject).not.toHaveBeenCalled();
  await open(); await act(async () => options()[1].click());
  expect(selectProject).toHaveBeenCalledExactlyOnceWith(projects[1]);
  expect(container.querySelector('[role="menu"]')).toBeNull();
  expect(document.activeElement).toBe(trigger());
});

it('supports keyboard selection, Escape and outside dismissal without switching projects', async () => {
  await render();
  await act(async () => trigger().dispatchEvent(new KeyboardEvent('keydown', { key:'ArrowDown', bubbles:true })));
  expect(document.activeElement?.getAttribute('role')).toBe('menuitemradio');
  await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key:'End', bubbles:true })));
  expect(document.activeElement?.textContent).toBe('添加项目');
  await act(async () => document.dispatchEvent(new KeyboardEvent('keydown', { key:'Escape', bubbles:true })));
  expect(document.activeElement).toBe(trigger());
  expect(container.querySelector('[role="menu"]')).toBeNull();
  await open(); await act(async () => document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles:true })));
  expect(container.querySelector('[role="menu"]')).toBeNull();
  expect(selectProject).not.toHaveBeenCalled(); expect(addProject).not.toHaveBeenCalled();
});

it('opens the folder picker for an empty workspace without inventing a project', async () => {
  await render([]); await open();
  expect(container.querySelector('[role="menuitemradio"]')).toBeNull();
  await act(async () => container.querySelector<HTMLButtonElement>('[role="menuitem"]')!.click());
  expect(addProject).toHaveBeenCalledOnce(); expect(selectProject).not.toHaveBeenCalled();
  expect(container.querySelector('[role="menu"]')).toBeNull();
});
