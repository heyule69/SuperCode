import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { AppBoundary } from './AppBoundary';
import { installFrontendDiagnostics } from './frontendDiagnostics';
import { restoreFrontendSettings } from './dataBackup';
import './styles.css';
import './settings.css';
import './client.css';
import './polish.css';
import './sidebar.css';
import './themes.css';
import './desktop.css';
import './meters.css';
import './model-menu.css';
import './extensions.css';
import './permission-menu.css';
import './followups.css';
import './new-chat.css';
import './workspace-layout.css';
import './chat-layout.css';

const disposeDiagnostics = installFrontendDiagnostics();
if (import.meta.hot) import.meta.hot.dispose(disposeDiagnostics);
const root = ReactDOM.createRoot(document.getElementById('root')!);
void restoreFrontendSettings().then(() => root.render(<React.StrictMode><AppBoundary><App /></AppBoundary></React.StrictMode>)).catch(error => {
  root.render(<main className="settings-modal"><h1>界面设置恢复未完成</h1><p role="alert">{String(error)}</p><button className="quiet-button" onClick={() => location.reload()}>重试</button></main>);
});
