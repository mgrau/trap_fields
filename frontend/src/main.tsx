import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ensureIsolation } from './isolation';
import './styles.css';

ensureIsolation().then((render) => {
  if (!render) return;
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
});
