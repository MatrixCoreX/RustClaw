import { useState } from 'react';
import type { AippCatalogItem } from '../../UI/src/types/api';
import { desktopOpenAipp } from './runtime';
import { friendlyError } from './errors';
export function DesktopAipp({app, lang}: {app: AippCatalogItem; lang: 'zh' | 'en'}) {
  const [error, setError] = useState('');
  return <section className="theme-panel-soft p-4"><p className="mb-3">{lang === 'zh' ? '此应用将在独立的受限窗口中打开。' : 'This app opens in a separate restricted window.'}</p><button className="theme-primary-btn px-4 py-2" onClick={() => void desktopOpenAipp(app.skill_name, lang).catch(e => setError(friendlyError(e)))}>{lang === 'zh' ? '打开应用' : 'Open app'}</button>{error && <p role="alert">{error}</p>}</section>;
}
