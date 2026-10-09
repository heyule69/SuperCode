import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { AppBoundary } from './AppBoundary';
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

ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><AppBoundary><App /></AppBoundary></React.StrictMode>);
