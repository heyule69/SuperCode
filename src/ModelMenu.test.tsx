import { expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModelMenu } from './ModelMenu';

it('keeps the official icon and selected display name on a historical imported conversation', () => {
  const official = { id: 'imported', agent: 'codex', name: 'OpenAI Official', officialAccount: true, model: 'official-model', models: ['official-model'], current: false, hasCredential: false };
  const html = renderToStaticMarkup(<ModelMenu value="official-model" models={[{ id: 'official-model', model: 'official-model', displayName: 'Selected Official Model', isDefault: true }]} source={{providerId:'openai',providerName:'OpenAI',mark:'O',available:true}} profiles={[official]} activeProfile={official} connectionId="imported" order={{ codex: ['@official'] }} agent="codex" busy={false} loading={false} choose={async () => {}} reload={() => {}} manage={() => {}} effort="" setEffort={() => {}} hasConversation />);
  expect(html).toContain('Selected Official Model');
  expect(html).toContain('/brands/codex-dark.png');
  expect(html).toContain('ChatGPT 官方账号');
});

it('replaces an unavailable stale Kimi badge and tooltip with an add supplier action', () => {
  const html = renderToStaticMarkup(<ModelMenu value="k3[1M]" models={[{id:'k3',model:'k3',displayName:'Kimi K3',isDefault:true}]} profiles={[]} connectionId="@local" source={{providerId:'kimi',providerName:'Kimi Code',mark:'K',available:false}} agent="claude" busy={false} loading={false} choose={async()=>{}} reload={()=>{}} manage={()=>{}} effort="" setEffort={()=>{}} hasConversation/>);
  expect(html).toContain('添加模型供应商');
  expect(html).not.toMatch(/Kimi|k3|model-provider-mark|model-chip-provider/);
});

it('does not invent a model from a saved ID when a configured supplier has no model catalog', () => {
  const profile = {id:'api',agent:'claude',name:'My API',model:null,current:true,hasCredential:true};
  const html = renderToStaticMarkup(<ModelMenu value="stale-model" models={[]} profiles={[profile]} activeProfile={profile} connectionId="api" source={{providerId:'custom',providerName:'My API',mark:'M',available:true}} agent="claude" busy={false} loading={false} choose={async()=>{}} reload={()=>{}} manage={()=>{}} effort="" setEffort={()=>{}} hasConversation/>);
  expect(html).toContain('选择模型');
  expect(html).not.toContain('stale-model');
  expect(html).not.toContain('添加模型供应商');
});
