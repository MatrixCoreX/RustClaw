import { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { DeviceForm } from './DeviceForm';
import { Downloads } from './Downloads';
import { LoginFields } from './LoginFields';
import { connectionLabel } from './connection-label';
import { friendlyError } from './errors';
import { forgetDeviceStorage } from './storage';
import { initializeRuntime, desktopLogout } from './runtime';
import type { LoginInput, LoginResult, Profile, SessionInfo } from './types';
import { PRODUCT_DISPLAY_NAME } from '../../UI/src/lib/product-identity';
import { version as desktopVersion } from '../package.json';
import { openWallet } from './wallet/store';
import './desktop.css';

document.title = PRODUCT_DISPLAY_NAME;
const root = createRoot(document.getElementById('root')!);

async function openConsole(info: SessionInfo) {
  initializeRuntime(info);
  const [{default: App}, {UiDialogProvider}] = await Promise.all([
    import('../../UI/src/App'), import('../../UI/src/components/UiDialogProvider'),
  ]);
  root.render(<><div className="desktop-device-bar"><span className="desktop-status-dot" /><strong>{info.profile.alias}</strong><span>{connectionLabel(info.profile.connection.kind)} · {info.profile.connection.kind === 'ssh' ? info.profile.connection.host : info.origin}</span><span className="desktop-role">{info.identity?.role === 'admin' ? '管理员' : '普通用户'}</span><button onClick={() => void desktopLogout()}>切换设备 / 断开</button></div><div className="desktop-console"><UiDialogProvider><App /></UiDialogProvider></div><Downloads /></>);
  document.addEventListener('click', e => {
    const anchor = (e.target as Element)?.closest?.('a');
    if (!anchor || anchor.download) return;
    const url = new URL(anchor.href, window.location.href);
    if (['http:', 'https:', 'mailto:'].includes(url.protocol) && url.hostname !== 'tauri.localhost') {
      e.preventDefault();
      if (e.isTrusted) void invoke('open_external', {url: url.href});
    }
  }, true);
}

function DeviceHome() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<Profile | null>(null);
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [sshSecret, setSshSecret] = useState('');
  const [sshKey, setSshKey] = useState<string | null>(null);
  const [sshKeyMode, setSshKeyMode] = useState(false);
  const [busy, setBusy] = useState(false);
  const running = useRef(false);
  const [connectingId, setConnectingId] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [failedProfileId, setFailedProfileId] = useState<string | null>(null);
  const errorElement = useRef<HTMLParagraphElement>(null);
  const [notice, setNotice] = useState('');
  const [theme, setTheme] = useState(() => localStorage.getItem('agent-runtime.monitor.themeMode') ?? 'light');
  const refresh = async () => {
    const profiles = await invoke<Profile[]>('profiles'); setProfiles(profiles);
  };
  useEffect(() => { document.documentElement.dataset.theme = theme; localStorage.setItem('agent-runtime.monitor.themeMode', theme); }, [theme]);
  useEffect(() => { void refresh().catch(e => setError(friendlyError(e))); }, []);
  useEffect(() => { if (error) errorElement.current?.scrollIntoView({block:'nearest'}); }, [error, failedProfileId]);
  const run = async (fn: () => Promise<void>) => {
    if (running.current) return;
    running.current = true; setBusy(true); setError(''); setFailedProfileId(null);
    try {await fn();} catch (e) {setError(friendlyError(e));}
    finally {running.current = false; setBusy(false);}
  };
  const clearSsh = () => {setSshSecret('');setSshKey(null);setSshKeyMode(false);};
  const chooseProfile = (profile: Profile) => {
    if (running.current) return;
    clearSsh(); setError(''); setFailedProfileId(null); setNotice('');
    if (profile.connection.kind === 'ssh') {setSelected(profile);return;}
    void run(async () => {
      setConnectingId(profile.id);
      try {setSession(await invoke<SessionInfo>('connect_device', {profileId:profile.id, sshSecret:''}));}
      catch (e) {setFailedProfileId(profile.id);throw e;}
      finally {setConnectingId(null);}
    });
  };
  const signIn = async (input: LoginInput | null, remember: boolean) => {
    const result = await invoke<LoginResult>('login', {sessionId: session!.id, input, remember});
    if (result.warning) {
      setNotice(friendlyError(result.warning)); setSession(result.session);
    } else await openConsole(result.session);
  };
  return <div className="desktop-home"><header className="desktop-header"><span className="desktop-mark">◈</span><div><strong>{PRODUCT_DISPLAY_NAME}</strong><span>桌面控制台 · 测试版 {desktopVersion}</span></div><button onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>{theme === 'light' ? '深色' : '浅色'}外观</button></header>
    <main className="desktop-home-content"><div className="desktop-intro"><span className="desktop-kicker">你的设备，安全连接</span><h1>{session ? '登录设备' : selected ? '连接设备' : '管理你的设备'}</h1><p>任务在设备上运行。关闭桌面窗口后，设备上的任务会继续。</p></div>
      {adding ? <DeviceForm onCancel={() => setAdding(false)} onAdded={p => {setProfiles(current => [...current, p]);setAdding(false);chooseProfile(p);}} /> : session ? <div className="desktop-card">
        <div className="desktop-verified">✓ {session.profile.connection.kind === 'local' ? '本机连接已建立' : '加密连接已建立'}</div><h2>{session.profile.alias}</h2><p>{connectionLabel(session.profile.connection.kind)} · {session.profile.connection.kind === 'ssh' ? session.profile.connection.host : session.origin}</p>
        {session.identity ? <><p className="desktop-note">{notice}</p><button type="button" className="primary" onClick={() => void openConsole(session)}>仅本次使用，进入控制台</button></> : <LoginFields key={session.id} sessionId={session.id} busy={busy} onLogin={(input, remember) => run(() => signIn(input, remember))} onDisconnect={() => void run(async () => {await invoke('disconnect_device');setSession(null);setSelected(null);})} />}
      </div> : selected ? <form className="desktop-card" onSubmit={e => {e.preventDefault(); void run(async () => {const value = sshSecret; const key = sshKey;setSshSecret('');setSshKey(null);setSession(await invoke<SessionInfo>('connect_device', {profileId:selected.id, sshSecret:sshKeyMode ? '' : value, sshKey:sshKeyMode ? key : null, sshKeyPassphrase:sshKeyMode ? value : null}));});}}>
        <h2>{selected.alias}</h2><p>{selected.connection.kind === 'ssh' ? selected.connection.host : selected.connection.origin}</p>
        {selected.connection.kind === 'ssh' && <>
          <label className="desktop-check"><input type="checkbox" checked={sshKeyMode} onChange={e => {setSshKeyMode(e.target.checked);setSshSecret('');setSshKey(null);}} />使用 SSH 私钥文件</label>
          {sshKeyMode && <label>SSH 私钥<input type="file" required onChange={async e => {const file=e.target.files?.[0];if(file && file.size <= 32768) setSshKey(await file.text()); else setError('请选择不超过 32 KB 的 SSH 私钥文件。');}} /><small>仅用于本次 SSH 登录，不保存在连接资料中。</small></label>}
          <label>{sshKeyMode ? '私钥口令（没有可留空）' : 'SSH 密码'}<input type="password" required={!sshKeyMode} autoComplete="off" value={sshSecret} onChange={e => setSshSecret(e.target.value)} /></label><small>用于建立加密隧道。之后还需登录设备账户。</small></>}
        <div className="desktop-actions"><button type="button" disabled={busy} onClick={() => {setSelected(null);clearSsh();setError('');}}>返回</button><button className="primary" disabled={busy}>{busy ? '正在连接…' : '连接'}</button></div>
      </form> : <>
        <div className="desktop-list-head"><h2>设备列表 <span>{profiles.length}</span></h2><div className="wallet-actions"><button disabled={busy} onClick={() => void run(async()=>{await openWallet();})}>本地资产账号</button><button className="primary" disabled={busy} onClick={() => {setAdding(true);setError('');}}>＋ 添加设备</button></div></div>
        {profiles.length === 0 ? <div className="desktop-card desktop-empty"><div className="desktop-empty-icon">⌘</div><h2>连接第一台设备</h2><p>可以自动查找本机服务，也可以添加局域网设备的 HTTPS 或 SSH 地址。</p><button className="primary" onClick={() => setAdding(true)}>添加设备</button></div> : <div className="desktop-device-list">{profiles.map(profile => <article className="desktop-card device-row" key={profile.id}><div><h2>{profile.alias}</h2><p>{profile.connection.kind === 'ssh' ? profile.connection.host : profile.connection.origin}</p><span className="desktop-badge" role="status">{connectionLabel(profile.connection.kind)} · {connectingId === profile.id ? '正在连接…' : '待连接'}</span>{error && failedProfileId === profile.id && <p className="desktop-error" role="alert" ref={errorElement}>{error}</p>}</div><div className="desktop-actions"><button className="subtle" disabled={busy} onClick={() => {
          if (!window.confirm(`忘记“${profile.alias}”并删除本机登录资料？设备上的账户、任务和数据会保留。`)) return;
          void run(async () => {await invoke('forget_profile', {profileId:profile.id});forgetDeviceStorage(localStorage, profile.id);forgetDeviceStorage(sessionStorage, profile.id);await refresh();});
        }}>忘记</button><button disabled={busy} onClick={() => chooseProfile(profile)}>{connectingId === profile.id ? '正在连接…' : '连接'}</button></div></article>)}</div>}
      </>}
      {error && !failedProfileId && <p className="desktop-error" role="alert" ref={errorElement}>{error}</p>}
      <footer>本机可用 HTTP，局域网设备使用 HTTPS 或 SSH。连接资料只保存在这台电脑上。</footer>
    </main></div>;
}

void invoke<SessionInfo | null>('current_session').then(info => {
  if (info?.identity) return openConsole(info);
  root.render(<DeviceHome />);
}).catch(error => root.render(<p role="alert">{friendlyError(error)}</p>));
