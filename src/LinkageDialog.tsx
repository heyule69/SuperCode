import { useRef, useState } from 'react';
import { Link2, X } from 'lucide-react';
import { LinkageSetup, type LinkageCatalog, type LinkageConfiguration } from './LinkageSetup';
import { useModalFocus } from './useModalFocus';

// Mount on demand so the existing home and conversation retain their layout.
// Only Apply changes the chat; dismissing the dialog discards local edits.
export function LinkageDialog({ value, catalogs, onApply, onClose, manage, disabled = false }: {
  value: LinkageConfiguration; catalogs: LinkageCatalog[]; onApply: (value: LinkageConfiguration) => void | Promise<void>;
  onClose: () => void; manage: (agentId: string) => void; disabled?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  const [working,setWorking]=useState(false);
  const [error,setError]=useState('');
  const panel = useRef<HTMLElement>(null);
  useModalFocus(true, panel, onClose,undefined,working,true);
  const choices = draft.mode === 'linked' ? [draft.controller, draft.executor] : [draft.controller];
  const valid = choices.every(choice => {
    const catalog = catalogs.find(item => item.agent.id === choice.agentId);
    return !catalog?.loading && !catalog?.error && !!choice.modelId && !!choice.connectionId && (catalog?.live || catalog?.connections.some(connection => connection.id === choice.connectionId
      && connection.models.some(model => model.id === choice.modelId)));
  });
  async function apply(){setWorking(true);setError('');try{await onApply(draft);}catch(error){setError(String(error));}finally{setWorking(false);}}
  return <div className="modal-backdrop linkage-dialog-backdrop" onMouseDown={event => { if (event.target === event.currentTarget&&!working) onClose(); }}>
    <section className="modal linkage-config-dialog" role="dialog" aria-modal="true" aria-labelledby="linkage-dialog-title" ref={panel} tabIndex={-1}>
      <div className="modal-header"><h2 id="linkage-dialog-title"><Link2 size={18}/>Agent 联动</h2><button type="button" className="icon-button" aria-label="关闭联动设置" disabled={working} onClick={onClose}><X size={18}/></button></div>
      <LinkageSetup value={draft} catalogs={catalogs} onChange={setDraft} manage={manage} disabled={disabled||working}/>
      {error?<p className="linkage-config-error" role="alert">{error}</p>:null}
      <footer className="linkage-dialog-footer"><button type="button" className="quiet-button" disabled={working} onClick={onClose}>{disabled ? '关闭' : '取消'}</button>
        {!disabled ? <button type="button" className="primary-button" disabled={!valid||working} onClick={()=>void apply()}>{working?'正在保存…':'完成'}</button> : null}
      </footer>
    </section>
  </div>;
}
