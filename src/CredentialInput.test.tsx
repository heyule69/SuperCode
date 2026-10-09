import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { CredentialInput } from './CredentialInput';

describe('saved credential presentation', () => {
  it('shows a read-only password mask and an explicit replace action', () => {
    const html=renderToStaticMarkup(<CredentialInput id="key" saved value="" onChange={() => { throw Error('mask must not be submitted'); }} />);
    expect(html).toContain('type="password"');
    expect(html).toContain('readOnly');
    expect(html).toContain('•'.repeat(32));
    expect(html).toContain('更换');
    expect(html).not.toContain('留空保留已保存的密钥');
  });
  it('keeps a new credential editable without a synthetic mask', () => {
    const html=renderToStaticMarkup(<CredentialInput id="key" saved={false} value="" onChange={() => {}} />);
    expect(html).not.toContain('readOnly');
    expect(html).not.toContain('•');
    expect(html).toContain('value=""');
  });
});
