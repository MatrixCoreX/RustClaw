import {computeUiBuildFingerprint} from './build-inputs.mjs';

const [uiDirectory, identityConfigPath, ...extra] = process.argv.slice(2);
if (!uiDirectory || !identityConfigPath || extra.length) {
  throw new Error('ui_build_fingerprint_arguments_invalid');
}
process.stdout.write(`${computeUiBuildFingerprint(uiDirectory, identityConfigPath)}\n`);
