import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import App from './App.tsx';
import {UiDialogProvider} from './components/UiDialogProvider.tsx';
import {RenderErrorBoundary} from './components/RenderErrorBoundary.tsx';
import {appStorageKey, PRODUCT_DISPLAY_NAME} from './lib/product-identity.ts';
import './index.css';

document.title = PRODUCT_DISPLAY_NAME;

function fallbackTranslate(zh: string, en: string): string {
  try { return window.localStorage.getItem(appStorageKey('monitor.lang')) === 'en' ? en : zh; }
  catch { return navigator.language.startsWith('zh') ? zh : en; }
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RenderErrorBoundary t={fallbackTranslate}>
    <UiDialogProvider>
      <App />
    </UiDialogProvider>
    </RenderErrorBoundary>
  </StrictMode>,
);
