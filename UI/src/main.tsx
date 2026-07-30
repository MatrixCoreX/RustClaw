import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import App from './App.tsx';
import {UiDialogProvider} from './components/UiDialogProvider.tsx';
import {PRODUCT_DISPLAY_NAME} from './lib/product-identity.ts';
import './index.css';

document.title = PRODUCT_DISPLAY_NAME;

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <UiDialogProvider>
      <App />
    </UiDialogProvider>
  </StrictMode>,
);
