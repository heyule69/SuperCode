import { expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModelMenu } from './ModelMenu';

it.each(['claude', 'codex', 'opencode', 'pi', 'gemini'])('shows Magpie names and advertised reasoning levels for %s without changing the routed model ID', agent => {
  const source = { providerId: 'magpie', providerName: 'Magpie', mark: 'M', available: true };
  const profile = { id: 'magpie', agent, name: 'Magpie', model: 'fixture/test-reasoner', current: true, hasCredential: true, modelSource: source };
  const html = renderToStaticMarkup(<ModelMenu value={profile.model} models={[{id:profile.model,model:profile.model,displayName:'Magpie 测试模型',isDefault:true,supportedReasoningEfforts:[{reasoningEffort:'low'},{reasoningEffort:'high'}]}]} source={source} profiles={[profile]} activeProfile={profile} connectionId={profile.id} agent={agent} busy={false} loading={false} choose={async()=>{}} reload={()=>{}} manage={()=>{}} effort="high" setEffort={()=>{}} hasConversation/>);
  expect(html).toContain('Magpie 测试模型');
  expect(html).toContain('推理强度：高');
  expect(html).toContain('/brands/magpie.png');
  expect(html).toContain('fixture/test-reasoner');
});

it.each([['high', '高'], ['max', '最高'], ['xhigh', '极高'], ['', '中']])('shows reasoning effort %s beside the selected model', (effort, label) => {
  const profile = { id:'api', agent:'claude', name:'API', model:'claude-opus-5-5', current:true, hasCredential:true };
  const html = renderToStaticMarkup(<ModelMenu value={profile.model} models={[{id:profile.model,model:profile.model,displayName:profile.model,isDefault:true,defaultReasoningEffort:'medium'}]} profiles={[profile]} activeProfile={profile} connectionId="api" agent="claude" busy={false} loading={false} choose={async()=>{}} reload={()=>{}} manage={()=>{}} effort={effort} setEffort={()=>{}} hasConversation/>);
  expect(html).toContain(`class="model-chip-effort" title="推理强度：${label}">${label}</span>`);
  expect(html).toContain(`推理强度：${label}${effort ? '' : '（默认）'}`);
});

it('does not show a reasoning badge for a model without reasoning controls', () => {
  const profile = { id:'api', agent:'pi', name:'API', model:'basic-model', current:true, hasCredential:true };
  const html = renderToStaticMarkup(<ModelMenu value={profile.model} models={[{id:profile.model,model:profile.model,displayName:profile.model,isDefault:true}]} profiles={[profile]} activeProfile={profile} connectionId="api" agent="pi" busy={false} loading={false} choose={async()=>{}} reload={()=>{}} manage={()=>{}} effort="" setEffort={()=>{}} hasConversation/>);
  expect(html).not.toContain('model-chip-effort');
});

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
