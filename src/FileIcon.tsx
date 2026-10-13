import { memo, useSyncExternalStore } from 'react';
import { fileIcon, fileIconCatalog, subscribeFileIconCatalog } from './fileIcons';
import './file-icons.css';

export const FileIcon = memo(function FileIcon({ name, directory = false, expanded = false, size = 16 }: { name: string; directory?: boolean; expanded?: boolean; size?: number }) {
  useSyncExternalStore(subscribeFileIconCatalog, fileIconCatalog, fileIconCatalog);
  const dark = fileIcon(name, directory, expanded);
  const light = fileIcon(name, directory, expanded, true);
  return <span className="file-kind-icon" aria-hidden="true" data-icon={dark.id} style={{ width: size, height: size }}>
    <img className="file-icon-dark" src={dark.url} alt="" width={size} height={size} draggable={false}/>
    {light.id !== dark.id ? <img className="file-icon-light" src={light.url} alt="" width={size} height={size} draggable={false}/> : null}
  </span>;
});
