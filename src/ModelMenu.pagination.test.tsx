// @vitest-environment happy-dom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it } from 'vitest';
import { ModelMenu } from './ModelMenu';

it('pages a large gateway directory and searches and navigates beyond the first 200 models', async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  const node = document.createElement('div'); document.body.append(node);
  const root = createRoot(node);
  const models = Array.from({ length: 1500 }, (_, index) => ({ id: `v/model-${index}`, model: `v/model-${index}`, displayName: `模型 ${index}`, isDefault: index === 1499 }));
  const profile = { id: 'magpie', agent: 'pi', name: 'Magpie', model: models[1499].id, models: models.map(model => model.id), current: true, hasCredential: true };
  try {
    await act(async () => root.render(<ModelMenu value={profile.model} models={models} catalogLoaded profiles={[profile]} activeProfile={profile} connectionId={profile.id} source={{providerId:'magpie',providerName:'Magpie',mark:'M',available:true}} agent="pi" busy={false} loading={false} choose={async () => {}} reload={() => {}} manage={() => {}} effort="" setEffort={() => {}} hasConversation/>));
    await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click());
    expect(node.querySelectorAll('button.model-option')).toHaveLength(50);
    expect(node.querySelector('.model-menu-scroll')!.textContent).toContain('模型 1499');
    expect(node.querySelector('.model-pagination')!.textContent).toContain('1500');
    await act(async () => node.querySelector<HTMLButtonElement>('[aria-label="模型上一页"]')!.click());
    expect(node.querySelector('.model-menu-scroll')!.textContent).toContain('模型 1400');
    const search = node.querySelector<HTMLInputElement>('[aria-label="搜索模型菜单"]')!;
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, 'model-1207'); search.dispatchEvent(new Event('input', { bubbles: true })); });
    expect(node.querySelectorAll('button.model-option')).toHaveLength(1);
    expect(node.querySelector('.model-menu-scroll')!.textContent).toContain('模型 1207');
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(search, ''); search.dispatchEvent(new Event('input', { bubbles: true })); });
    const first = node.querySelector<HTMLButtonElement>('button.model-option')!;
    await act(async () => { first.focus(); first.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })); });
    expect(document.activeElement?.textContent).toContain('模型 1499');
    expect(node.querySelectorAll('button.model-option')).toHaveLength(50);
  } finally { await act(async () => root.unmount()); node.remove(); }
});
