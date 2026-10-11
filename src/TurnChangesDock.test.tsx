// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import TurnChangesDock from './TurnChangesDock';
import { currentTurnChanges } from './turnChanges';
import { mergeEvent } from './events';
import type { Message } from './types';

const patch = 'diff --git a/src/file.ts b/src/file.ts\n@@ -1 +1 @@\n-old\n+new';
const user = (id:string):Message => ({id,seq:1,sessionId:'s',role:'user',kind:'userMessage',text:id,data:{}});
function running() { return mergeEvent([user('first')],{method:'turn/started',params:{turn:{id:'turn1'}}},'s'); }
function update(messages:Message[],diff=patch) { return mergeEvent(messages,{method:'turn/diff/updated',params:{turnId:'turn1',diff}},'s'); }
let root:Root, host:HTMLDivElement;
const showDiff=vi.fn();
beforeEach(()=>{(globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;host=document.createElement('div');document.body.append(host);root=createRoot(host);showDiff.mockClear();});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});
async function render(messages:Message[],activeTurnId?:string) { await act(async()=>root.render(<TurnChangesDock messages={messages} activeTurnId={activeTurnId} showDiff={showDiff}/>)); }
it('updates cumulative live snapshots once, keeps the finished result, and resets on the next prompt',async()=>{
  let messages=update(running()); await render(messages,'turn1');
  expect(host.textContent).toContain('1 个文件已更改');
  expect(host.textContent).toContain('+1');
  messages=update(messages,patch+'\n+extra');await render(messages,'turn1');
  expect(host.textContent).toContain('+2');
  expect(host.textContent).not.toContain('+3');
  messages=mergeEvent(messages,{method:'turn/completed',params:{turn:{id:'turn1',status:'completed'}}},'s');await render(messages);
  expect(host.textContent).toContain('1 个文件已更改');
  await render([...messages,user('next')]);expect(host.textContent).toBe('');
  await render(messages,'turn2');expect(host.textContent).toBe('');
});
it('reads each file snapshot from the current turn and removes a fully reverted file',()=>{
  const snapshot:Message={id:'change-turn1:src/file.ts',seq:3,sessionId:'s',role:'activity',kind:'turnDiff',text:patch,data:{turnId:'turn1',path:'src/file.ts'}};
  const other:Message={...snapshot,id:'other',text:patch.replaceAll('src/file.ts','other.ts'),data:{turnId:'turn1',path:'other.ts'}};
  expect(currentTurnChanges([...running(),snapshot,other]).files.map(file=>file.path)).toEqual(['src/file.ts','other.ts']);
  expect(currentTurnChanges([...running(),{...snapshot,text:''},other]).files.map(file=>file.path)).toEqual(['other.ts']);
  expect(currentTurnChanges(update(running(), '')).files).toEqual([]);
});
it('opens the current file patch and closes its menu with Escape',async()=>{
  await render(update(running()),'turn1');
  await act(async()=>host.querySelector<HTMLButtonElement>('.turn-change-chip')!.click());
  expect(host.querySelector('[role="menu"]')).not.toBeNull();
  await act(async()=>document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
  expect(host.querySelector('[role="menu"]')).toBeNull();
  expect(document.activeElement).toBe(host.querySelector('.turn-change-chip'));
  await act(async()=>host.querySelector<HTMLButtonElement>('.turn-change-chip')!.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="查看本轮 src/file.ts 变更"]')!.click());
  expect(showDiff).toHaveBeenCalledWith('本轮变更 · src/file.ts',expect.stringContaining('-old\n+new'));
  expect(host.querySelector('[role="menu"]')).toBeNull();
});
