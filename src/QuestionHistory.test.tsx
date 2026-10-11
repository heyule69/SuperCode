import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { QuestionHistory } from './QuestionHistory';
import type { Message } from './types';

const questions = [
  { header: '界面形态', question: '插件的界面形态用哪种？', options: [{ label: '全页 Dashboard（推荐）', description: '图片视频瀑布流预览、勾选、过滤、批量下载。' }, { label: 'Popup 弹窗', description: '适合轻量使用。' }] },
  { id: 'types', question: '支持哪些内容？', multiSelect: true, options: [{ label: '图片' }, { label: '视频' }] },
];
const message = (data: Message['data'], text = ''): Message => ({ id: 'ask', seq: 1, sessionId: 's', role: 'tool', kind: 'claudeToolCall', text, data: { tool: 'AskUserQuestion', ...data } });
const render = (m: Message) => renderToStaticMarkup(<QuestionHistory message={m} />);

describe('readable question history', () => {
  it('displays multiple questions with full option descriptions instead of protocol parameters', () => {
    const html = render(message({ arguments: { questions }, status: 'inProgress' }));
    for (const value of ['插件的界面形态用哪种？', '界面形态', '全页 Dashboard（推荐）', '图片视频瀑布流预览、勾选、过滤、批量下载。', '问题 2 / 2', '可多选']) expect(html).toContain(value);
    expect(html).not.toContain('<pre');
    expect(html).not.toContain('&quot;questions&quot;');
    expect(html).not.toContain('multiSelect');
    expect(html).not.toContain('输入参数');
  });

  it('waits for a complete streamed question and supports stored JSON arguments', () => {
    const preparing = message({ arguments: {}, inputText: '{"questions":[{"question":', status: 'preparing' });
    const html = render(preparing);
    expect(html).toContain('正在生成问题…');
    expect(html).not.toContain('&quot;questions&quot;');
    for (const data of [{ arguments: {}, inputText: JSON.stringify({ questions }) }, { arguments: JSON.stringify({ questions }) }]) expect(render(message(data))).toContain('插件的界面形态用哪种？');
  });

  it('shows answers keyed by question text or request ID, and marks actual selected options', () => {
    const html = render(message({ arguments: { questions } }, JSON.stringify({ answers: { [questions[0].question]: 'Popup 弹窗', types: { answers: ['图片', '视频'] } } })));
    expect(html.match(/class="question-history-answer"/g)).toHaveLength(2);
    expect(html.match(/class="is-selected"/g)).toHaveLength(3);
    expect(html).toContain('图片、视频');
    expect(html).not.toContain('&quot;answers&quot;');
  });

  it('preserves custom answers and text cancellation reasons without making them selectable', () => {
    const answered = render(message({ arguments: { questions, answers: { [questions[0].question]: '单独打开一个窗口' } }, status: 'completed' }));
    expect(answered).toContain('单独打开一个窗口');
    expect(answered).not.toContain('class="is-selected"');
    const cancelled = render(message({ arguments: { questions }, status: 'declined' }, '用户跳过了这些问题。'));
    expect(cancelled).toContain('用户跳过了这些问题。');
    expect(cancelled).not.toContain('<pre');
  });

  it('hides secret answers and escapes user-facing question content', () => {
    const html = render(message({ arguments: { questions: [{ id: 'secret', isSecret: true, question: '<img src=x onerror=alert(1)>密码？' }] } }, JSON.stringify({ answers: { secret: { answers: ['secret-value'] } } })));
    expect(html).toContain('内容已隐藏');
    expect(html).not.toContain('secret-value');
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img');
  });

  it('handles incomplete historical data and never falls back to displaying raw JSON', () => {
    for (const data of [null, { arguments: { questions: [null, {}, { question: 123, options: null }] } }, { arguments: '{broken json' }]) {
      const html = render(message(data, '{"internal":"parameter"}'));
      expect(html).toContain('未收到问题详情');
      expect(html).not.toContain('parameter');
      expect(html).not.toContain('<pre');
    }
  });
});
