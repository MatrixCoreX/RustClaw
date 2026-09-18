import {defineConfig, mergeConfig} from '../../UI/node_modules/vite/dist/node/index.js';
import desktop from '../vite.config';
import fs from 'node:fs';
import path from 'node:path';
export default defineConfig(env => mergeConfig(typeof desktop==='function'?desktop(env):desktop, {
  plugins: [{name:'android-viewport',
    transform(code: string, id: string) {
      if (id.split('?')[0].endsWith('/frontend/main.tsx')) return `import ${JSON.stringify(path.resolve(__dirname, 'export.ts'))};\n${code}`;
      if (id.split('?')[0].endsWith('/frontend/i18n.tsx')) return code.replace(
        'return productCopy(language === "zh" ? zh : en ?? translations[zh] ?? text);',
        'return productCopy(language === "zh" ? zh : en ?? translations[zh] ?? text).replaceAll("桌面本地", "本机").replaceAll("桌面端", "应用").replaceAll("桌面控制台", "移动控制台").replaceAll("Desktop console", "Mobile console").replaceAll("desktop app", "app").replaceAll("桌面窗口", "应用").replaceAll("这台电脑", "本设备").replaceAll("桌面账号", "本机账号").replaceAll("desktop account", "local account");');
    }, transformIndexHtml(html: string) {
    return html.replace('<html ', '<html data-platform="android" ').replace('initial-scale=1', 'viewport-fit=cover,initial-scale=1')
      .replace('</head>', `<style>${fs.readFileSync(path.resolve(__dirname,'mobile.css'),'utf8')}</style></head>`);
  }}],
  build: {outDir: 'android/dist', target: 'chrome111', cssTarget: 'chrome111'},
}));
