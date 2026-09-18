import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const android=path.join(root,'android');
function run(command,args,options={}) {
  const result=spawnSync(command,args,{cwd:root,env:process.env,stdio:'inherit',...options});
  if(result.status!==0) throw new Error(`${command} failed (${result.status})`);
  return result;
}
for(const key of ['JAVA_HOME','ANDROID_HOME','NDK_HOME']) {
  if(!process.env[key] || !fs.existsSync(process.env[key])) throw new Error(`${key} is required`);
}
const config=process.env.APP_PRODUCT_IDENTITY_CONFIG || path.resolve(root,'../configs/product_identity.toml');
const identity=JSON.parse(run('cargo',['run','--quiet','--locked','--manifest-path','scripts/identity/Cargo.toml'],{
  env:{...process.env,APP_PRODUCT_IDENTITY_CONFIG:config},stdio:['ignore','pipe','inherit'],encoding:'utf8',
}).stdout);
const generated=path.join(android,'.tools');fs.mkdirSync(generated,{recursive:true});
const projection=path.join(generated,'identity.json');
fs.writeFileSync(projection,JSON.stringify({productName:identity.display_name}));
fs.writeFileSync(path.join(generated,'product-identity.json'),JSON.stringify(identity));
process.env.ANDROID_SIGNING_KEYSTORE ||= path.join(android,'.signing/release.p12');
process.env.ANDROID_SIGNING_PASSWORD_FILE ||= path.join(android,'.signing/password');
for(const name of ['ANDROID_SIGNING_KEYSTORE','ANDROID_SIGNING_PASSWORD_FILE']) {
  if(!fs.existsSync(process.env[name])) throw new Error(`${name} is required for a release APK`);
}
const args=process.argv.slice(2);
run(process.execPath,['node_modules/@tauri-apps/cli/tauri.js','android','build','--apk','--config',projection,...args]);
