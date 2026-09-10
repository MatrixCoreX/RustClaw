import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { DeviceForm } from './DeviceForm';
import { Downloads } from './Downloads';
import { friendlyError } from './errors';
import { forgetDeviceStorage } from './storage';
import { initializeRuntime, desktopLogout } from './runtime';
import type { LoginResult, Profile, SessionInfo } from './types';
import { PRODUCT_DISPLAY_NAME } from '../../UI/src/lib/product-identity';
import { version as desktopVersion } from '../package.json';
import './desktop.css';

document.title = PRODUCT_DISPLAY_NAME;
const root = createRoot(document.getElementById('root')!);

async function openConsole(info: SessionInfo) {
  initializeRuntime(info);
  const [{default: App}, {UiDialogProvider}] = await Promise.all([
    import('../../UI/src/App'), import('../../UI/src/components/UiDialogProvider'),
  ]);
  root.render(<><div className="desktop-device-bar"><span className="desktop-status-dot" /><strong>{info.profile.alias}</strong><span>{info.profile.connection.kind.toUpperCase()} · {info.profile.connection.kind === 'ssh' ? info.profile.connection.host : info.origin}</span><span className="desktop-role">{info.identity?.role === 'admin' ? '管理员' : '普通用户'}</span><button onClick={() => void desktopLogout()}>切换设备 / 断开</button></div><div className="desktop-console"><UiDialogProvider><App /></UiDialogProvider></div><Downloads /></>);
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
  const [mode, setMode] = useState<'password' | 'key'>('password');
  const [username, setUsername] = useState('');
  const [secret, setSecret] = useState('');
  const [remember, setRemember] = useState(false);
  const [savedProfiles, setSavedProfiles] = useState<string[]>(() => JSON.parse(localStorage.getItem('agent-runtime.desktop.saved-profile-ids') || '[]'));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [theme, setTheme] = useState(() => localStorage.getItem('agent-runtime.monitor.themeMode') ?? 'light');
  const refresh = async () => {
    const profiles = await invoke<Profile[]>('profiles'); setProfiles(profiles);
    setSavedProfiles(profiles.filter(p => p.saved_login).map(p => p.id));
  };
  useEffect(() => { document.documentElement.dataset.theme = theme; localStorage.setItem('agent-runtime.monitor.themeMode', theme); }, [theme]);
  useEffect(() => { void refresh().catch(e => setError(friendlyError(e))); }, []);
  const run = async (fn: () => Promise<void>) => {setBusy(true); setError(''); try {await fn();} catch (e) {setError(friendlyError(e));} finally {setBusy(false);}};
  const signIn = async (saved = false) => {
    const input = saved ? null : {mode, username, secret};
    setSecret('');
    const result = await invoke<LoginResult>('login', {sessionId: session!.id, input, remember: saved ? false : remember});
    if (result.remembered) {
      const next = [...new Set([...savedProfiles, session!.profile.id])];
      localStorage.setItem('agent-runtime.desktop.saved-profile-ids', JSON.stringify(next)); setSavedProfiles(next);
    }
    if (result.warning) {
      setNotice(friendlyError(result.warning)); setSession(result.session);
    } else await openConsole(result.session);
  };
  return <div className="desktop-home"><header className="desktop-header"><span className="desktop-mark">◈</span><div><strong>{PRODUCT_DISPLAY_NAME}</strong><span>桌面控制台 · 测试版 {desktopVersion}</span></div><button onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>{theme === 'light' ? '深色' : '浅色'}外观</button></header>
    <main className="desktop-home-content"><div className="desktop-intro"><span className="desktop-kicker">你的设备，安全连接</span><h1>{session ? '登录设备' : selected ? '连接设备' : '管理你的设备'}</h1><p>任务在设备上运行。关闭桌面窗口后，设备上的任务会继续。</p></div>
      {adding ? <DeviceForm onCancel={() => setAdding(false)} onAdded={p => {setAdding(false);setSelected(p);void refresh();}} /> : session ? <form className="desktop-card" onSubmit={e => {e.preventDefault(); void run(() => signIn());}}>
        <div className="desktop-verified">✓ 加密连接已建立</div><h2>{session.profile.alias}</h2><p>{session.profile.connection.kind.toUpperCase()} · {session.profile.connection.kind === 'ssh' ? session.profile.connection.host : session.origin}</p>
        {session.identity ? <><p className="desktop-note">{notice}</p><button type="button" className="primary" onClick={() => void openConsole(session)}>仅本次使用，进入控制台</button></> : <>
          <div className="desktop-tabs"><button type="button" className={mode === 'password' ? 'selected' : ''} onClick={() => {setMode('password');setSecret('');}}>用户名与密码</button><button type="button" className={mode === 'key' ? 'selected' : ''} onClick={() => {setMode('key');setSecret('');}}>用户 Key</button></div>
          {mode === 'password' && <label>设备账户<input required autoComplete="username" value={username} onChange={e => setUsername(e.target.value)} /></label>}
          <label>{mode === 'key' ? '用户 Key' : '密码'}<input required type="password" autoComplete="off" value={secret} onChange={e => setSecret(e.target.value)} /></label>
          <label className="desktop-check"><input type="checkbox" checked={remember} onChange={e => setRemember(e.target.checked)} />保存到系统凭据库</label><small>不勾选时仅当前会话使用。设备账户权限仍由设备管理。</small>
          <div className="desktop-actions"><button type="button" disabled={busy} onClick={() => void run(async () => {await invoke('disconnect_device');setSession(null);setSelected(null);})}>断开</button>{savedProfiles.includes(session.profile.id) && <button type="button" disabled={busy} onClick={() => void run(() => signIn(true))}>使用已保存的登录</button>}<button className="primary" disabled={busy}>{busy ? '正在登录…' : '登录设备'}</button></div>
        </>}
      </form> : selected ? <form className="desktop-card" onSubmit={e => {e.preventDefault(); void run(async () => {const value = sshSecret; const key = sshKey;setSshSecret('');setSshKey(null);setSession(await invoke<SessionInfo>('connect_device', {profileId:selected.id, sshSecret:sshKeyMode ? '' : value, sshKey:sshKeyMode ? key : null, sshKeyPassphrase:sshKeyMode ? value : null}));});}}>
        <h2>{selected.alias}</h2><p>{selected.connection.kind === 'https' ? selected.connection.origin : selected.connection.host}</p>
        {selected.connection.kind === 'ssh' ? <>
          <label className="desktop-check"><input type="checkbox" checked={sshKeyMode} onChange={e => {setSshKeyMode(e.target.checked);setSshSecret('');setSshKey(null);}} />使用 SSH 私钥文件</label>
          {sshKeyMode && <label>SSH 私钥<input type="file" required onChange={async e => {const file=e.target.files?.[0];if(file && file.size <= 32768) setSshKey(await file.text()); else setError('请选择不超过 32 KB 的 SSH 私钥文件。');}} /><small>仅用于本次 SSH 登录，不保存在连接资料中。</small></label>}
          <label>{sshKeyMode ? '私钥口令（没有可留空）' : 'SSH 密码'}<input type="password" required={!sshKeyMode} autoComplete="off" value={sshSecret} onChange={e => setSshSecret(e.target.value)} /></label><small>用于建立加密隧道。之后还需登录设备账户。</small></> : <p className="desktop-note">先验证证书，再进入登录。连接失败时不会切换到 HTTP。</p>}
        <div className="desktop-actions"><button type="button" disabled={busy} onClick={() => {setSelected(null);setError('');}}>返回</button><button className="primary" disabled={busy}>{busy ? '正在核验并连接…' : '建立安全连接'}</button></div>
      </form> : <>
        <div className="desktop-list-head"><h2>设备列表 <span>{profiles.length}</span></h2><button className="primary" onClick={() => {setAdding(true);setError('');}}>＋ 添加设备</button></div>
        {profiles.length === 0 ? <div className="desktop-card desktop-empty"><div className="desktop-empty-icon">⌘</div><h2>连接第一台设备</h2><p>准备设备的 HTTPS 地址，或 SSH 地址与已核对的主机指纹。</p><button className="primary" onClick={() => setAdding(true)}>添加设备</button></div> : <div className="desktop-device-list">{profiles.map(profile => <article className="desktop-card device-row" key={profile.id}><div><h2>{profile.alias}</h2><p>{profile.connection.kind === 'https' ? profile.connection.origin : profile.connection.host}</p><span className="desktop-badge">{profile.connection.kind.toUpperCase()} · 待连接</span></div><div className="desktop-actions"><button className="subtle" disabled={busy} onClick={() => {
          if (!window.confirm(`忘记“${profile.alias}”并删除本机登录资料？设备上的账户、任务和数据会保留。`)) return;
          void run(async () => {await invoke('forget_profile', {profileId:profile.id, deleteSavedLogin:savedProfiles.includes(profile.id)});forgetDeviceStorage(localStorage, profile.id);forgetDeviceStorage(sessionStorage, profile.id);const next = savedProfiles.filter(id => id !== profile.id);setSavedProfiles(next);localStorage.setItem('agent-runtime.desktop.saved-profile-ids', JSON.stringify(next));await refresh();});
        }}>忘记</button><button onClick={() => {setSelected(profile);setError('');}}>连接</button></div></article>)}</div>}
      </>}
      {error && <p className="desktop-error" role="alert">{error}</p>}
      <footer>HTTPS 与 SSH 均独立可用。连接资料只保存在这台电脑上。</footer>
    </main></div>;
}

void invoke<SessionInfo | null>('current_session').then(info => {
  if (info?.identity) return openConsole(info);
  root.render(<DeviceHome />);
}).catch(error => root.render(<p role="alert">{friendlyError(error)}</p>));
