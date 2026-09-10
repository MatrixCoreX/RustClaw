import path from 'node:path';
import { defineConfig, mergeConfig } from '../UI/node_modules/vite/dist/node/index.js';
import sharedConfig from '../UI/vite.config';
import { sharedUiAdapter } from './scripts/shared-ui-adapter';

export default defineConfig((env) => {
  const base = typeof sharedConfig === 'function' ? sharedConfig(env) : sharedConfig;
  return mergeConfig(base, {
    root: __dirname,
    base: './',
    publicDir: '../UI/public',
    plugins: [sharedUiAdapter(__dirname)],
    resolve: {alias: {
      react: path.resolve(__dirname, '../UI/node_modules/react'),
      'react-dom': path.resolve(__dirname, '../UI/node_modules/react-dom'),
    }},
    build: {outDir: 'dist', emptyOutDir: true, rollupOptions: {input: {
      index: path.resolve(__dirname, 'index.html'), aipp: path.resolve(__dirname, 'aipp.html'),
    }}},
  });
});
