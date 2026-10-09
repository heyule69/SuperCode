import { expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ModelMenu } from './ModelMenu';

it('keeps the official icon and selected display name on a historical imported conversation', () => {
  const official = { id: 'imported', agent: 'codex', name: 'OpenAI Official', officialAccount: true, model: 'official-model', models: ['official-model'], current: false, hasCredential: false };
  const html = renderToStaticMarkup(<ModelMenu value="official-model" models={[{ id: 'official-model', model: 'official-model', displayName: 'Selected Official Model', isDefault: true }]} profiles={[official]} activeProfile={official} connectionId="imported" order={{ codex: ['@official'] }} agent="codex" busy={false} loading={false} choose={async () => {}} reload={() => {}} manage={() => {}} effort="" setEffort={() => {}} hasConversation />);
  expect(html).toContain('Selected Official Model');
  expect(html).toContain('/brands/codex-dark.png');
  expect(html).toContain('ChatGPT 官方账号');
});
