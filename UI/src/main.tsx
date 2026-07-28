import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import App from './App.tsx';
import {UiDialogProvider} from './components/UiDialogProvider.tsx';
import './index.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <UiDialogProvider>
      <App />
    </UiDialogProvider>
  </StrictMode>,
);
