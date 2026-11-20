import React from 'react';
import ReactDOM from 'react-dom/client';
import { LinkagePreview } from './LinkagePreview';
import '../styles.css';
import '../client.css';
import '../polish.css';
import '../sidebar.css';
import '../themes.css';
import '../desktop.css';
import '../model-menu.css';
import '../permission-menu.css';
import '../new-chat.css';
import '../workspace-layout.css';
import '../chat-layout.css';
import './preview.css';

const theme = new URLSearchParams(location.search).get('theme');
document.documentElement.dataset.theme = theme === 'dark' || theme === 'graphite' || theme === 'sand' ? theme : 'light';
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><LinkagePreview/></React.StrictMode>);
