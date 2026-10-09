import { useEffect, useRef, useState } from 'react';

// The mask is UI state only. It is never a credential and never reaches onChange.
export function CredentialInput({ saved, value, onChange, id, disabled, revision = 0 }: { saved: boolean; value: string; onChange: (value: string) => void; id: string; disabled?: boolean; revision?: number }) {
  const [editing, setEditing] = useState(!saved);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { setEditing(!saved); }, [saved, revision]);
  const masked = saved && !editing;
  return <div className="credential-input"><input ref={input} id={id} type="password" autoComplete="new-password" spellCheck={false} readOnly={masked} disabled={disabled}
    value={masked ? '•'.repeat(32) : value} aria-label={masked ? 'API Key，已保存' : 'API Key'}
    onChange={e => { if (!masked) onChange(e.target.value); }} placeholder={saved ? '输入新 Key，留空保留原 Key' : 'API Key'} />
    {saved ? <button type="button" disabled={disabled} onClick={() => { if (editing) { onChange(''); setEditing(false); } else { setEditing(true); requestAnimationFrame(() => input.current?.focus()); } }}>{editing ? '取消更换' : '更换'}</button> : null}
  </div>;
}
