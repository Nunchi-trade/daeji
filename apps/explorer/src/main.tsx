import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';

import '@fontsource-variable/jetbrains-mono';
import '@fontsource-variable/fraunces/wght-italic.css';

import './design/reset.css';
import './design/tokens.css';
import './design/typography.css';
import './design/atmosphere.css';

const root = document.getElementById('root');
if (!root) throw new Error('Root element not found');

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
