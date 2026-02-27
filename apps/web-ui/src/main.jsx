import './runtime/react-devtools-hook.js';
import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App.jsx';
import { resolveStrictModeEnabled } from './runtime/runtime-flags.js';
import './styles.css';

const strictModeEnabled = resolveStrictModeEnabled(import.meta.env);
const appTree = strictModeEnabled
  ? (
      <React.StrictMode>
        <App />
      </React.StrictMode>
    )
  : <App />;

createRoot(document.getElementById('root')).render(appTree);
