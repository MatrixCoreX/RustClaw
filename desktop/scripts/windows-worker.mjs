import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';

export function buildWindowsWorker(root, args) {
  const index=args.indexOf('--target');
  const target=(index>=0 ? args[index+1] : args.find(arg=>arg.startsWith('--target='))?.slice(9))
    || process.env.DESKTOP_NATIVE_TARGET || 'x86_64-pc-windows-msvc';
  if(target!=='x86_64-pc-windows-msvc') throw new Error('wallet_worker_target_unsupported');
  const base=path.resolve(process.env.CARGO_TARGET_DIR || path.join(root,'target'));
  const cache=path.join(base,'wallet-worker');
  const result=spawnSync('cargo',['build','--locked','--release','--manifest-path',
    path.join(root,'platforms/windows/worker/Cargo.toml'),'--target',target],{
      cwd:root,stdio:'inherit',env:{...process.env,CARGO_TARGET_DIR:cache},
    });
  if(result.status!==0) throw new Error('wallet_worker_build_failed');
  const binary=path.join(cache,target,'release/agent-vault.exe');
  fs.copyFileSync(binary,path.join(root,`.build/agent-vault-${target}.exe`));
  const output=path.join(base,...(index>=0 || args.some(arg=>arg.startsWith('--target=')) ? [target] : []),args.includes('--dev')?'debug':'release');
  fs.mkdirSync(output,{recursive:true});
  fs.copyFileSync(binary,path.join(output,'agent-vault.exe'));
  return ['.build/agent-vault'];
}
