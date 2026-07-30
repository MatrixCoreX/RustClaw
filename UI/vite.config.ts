import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import fs from 'fs';
import path from 'path';
import {defineConfig, loadEnv} from 'vite';

function productIdentityConfig(): {displayName: string} {
  const configPath = process.env.APP_PRODUCT_IDENTITY_CONFIG
    ? path.resolve(process.env.APP_PRODUCT_IDENTITY_CONFIG)
    : path.resolve(__dirname, '../configs/product_identity.toml');
  const raw = fs.readFileSync(configPath, 'utf8');
  const readString = (key: string): string | undefined => {
    const match = raw.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"]*)"`, 'm'));
    return match?.[1]?.trim() || undefined;
  };
  const schemaVersion = raw.match(/^\s*schema_version\s*=\s*(\d+)/m)?.[1];
  const displayName = readString('display_name');
  const releaseArtifactId = readString('release_artifact_id');
  const releaseRepository = readString('release_repository');
  const splashImage = readString('small_screen_splash_image');
  if (schemaVersion !== '1') throw new Error(`Unsupported product identity schema in ${configPath}`);
  if (!displayName) throw new Error(`Missing display_name in ${configPath}`);
  if (!releaseArtifactId || !/^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/.test(releaseArtifactId)) {
    throw new Error(`Invalid release_artifact_id in ${configPath}`);
  }
  if (!releaseRepository || !/^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(releaseRepository)) {
    throw new Error(`Invalid release_repository in ${configPath}`);
  }
  if (!splashImage || splashImage === '.' || splashImage === '..' || /[\\/]/.test(splashImage)) {
    throw new Error(`Invalid small_screen_splash_image in ${configPath}`);
  }
  return {displayName};
}

export default defineConfig(({mode}) => {
  const env = loadEnv(mode, '.', '');
  const identity = productIdentityConfig();
  return {
    plugins: [react(), tailwindcss()],
    define: {
      'process.env.GEMINI_API_KEY': JSON.stringify(env.GEMINI_API_KEY),
      __APP_DISPLAY_NAME__: JSON.stringify(identity.displayName),
    },
    resolve: {
      alias: {
        '@': path.resolve(__dirname, '.'),
      },
    },
    server: {
      // HMR is disabled in AI Studio via DISABLE_HMR env var.
      // Do not modifyâfile watching is disabled to prevent flickering during agent edits.
      hmr: process.env.DISABLE_HMR !== 'true',
    },
  };
});
