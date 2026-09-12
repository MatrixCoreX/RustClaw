import { copy, LanguageToggle, useLanguage } from "./i18n";
import { lazy, Suspense, useEffect, useRef, useState } from 'react';
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
import { selectAccount } from './wallet/store';
import { HomeAccounts } from './standalone/HomeAccounts';
import { initialTheme, ThemeToggle } from './ThemeToggle';
const AssetWorkspace = lazy(() => import('./standalone/Workspace'));
import './desktop.css';

document.title = PRODUCT_DISPLAY_NAME;
document.documentElement.dataset.theme = initialTheme();
const root = createRoot(document.getElementById('root')!);

async function openConsole(info: SessionInfo) {
  initializeRuntime(info);
  const [{default: App}, {UiDialogProvider}] = await Promise.all([
    import('../../UI/src/App'), import('../../UI/src/components/UiDialogProvider'),
  ]);
  root.render(<><DeviceBar info={info} /><div className="desktop-console"><UiDialogProvider><App /></UiDialogProvider></div><Downloads /></>);
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

function DeviceBar({ info }: { info: SessionInfo }) {
  useLanguage();
  return <div className="desktop-device-bar"><span className="desktop-status-dot" /><strong>{info.profile.alias}</strong><span>{connectionLabel(info.profile.connection.kind)} · {info.profile.connection.kind === 'ssh' ? info.profile.connection.host : info.origin}</span><span className="desktop-role">{info.identity?.role === 'admin' ? copy("管理员") : copy("普通用户")}</span><button onClick={() => void desktopLogout()}>{copy("切换设备 / 断开")}</button></div>;
}

function DeviceHome() {
  useLanguage();
  const [assetPage, setAssetPage] = useState<'assets' | 'bancor' | null>(null);
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
  const [theme, setTheme] = useState<string>(initialTheme);
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
  if (assetPage) return <Suspense fallback={<div className="desktop-home desktop-home-content">{copy("正在打开资产页面…")}</div>}><AssetWorkspace initialPage={assetPage} onHome={() => { setTheme(document.documentElement.dataset.theme || 'dark'); setAssetPage(null); }} /></Suspense>;
  return <div className="desktop-home"><header className="desktop-header"><span className="desktop-mark">◈</span><div><strong>{PRODUCT_DISPLAY_NAME}</strong><span>{copy("桌面控制台 · 测试版")} {desktopVersion}</span></div><LanguageToggle /><ThemeToggle theme={theme} onToggle={() => setTheme(theme === 'light' ? 'dark' : 'light')} /></header>
    <main className="desktop-home-content"><div className="desktop-intro"><span className="desktop-kicker">{copy("你的设备，安全连接")}</span><h1>{session ? copy("登录设备") : selected ? copy("连接设备") : copy("管理你的设备")}</h1><p>{copy("任务在设备上运行。关闭桌面窗口后，设备上的任务会继续。")}</p></div>
      {adding ? <DeviceForm onCancel={() => setAdding(false)} onAdded={p => {setProfiles(current => [...current, p]);setAdding(false);chooseProfile(p);}} /> : session ? <div className="desktop-card">
        <div className="desktop-verified">✓ {session.profile.connection.kind === 'local' ? copy("本机连接已建立") : copy("加密连接已建立")}</div><h2>{session.profile.alias}</h2><p>{connectionLabel(session.profile.connection.kind)} · {session.profile.connection.kind === 'ssh' ? session.profile.connection.host : session.origin}</p>
        {session.identity ? <><p className="desktop-note">{copy(notice)}</p><button type="button" className="primary" onClick={() => void openConsole(session)}>{copy("仅本次使用，进入控制台")}</button></> : <LoginFields key={session.id} sessionId={session.id} busy={busy} onLogin={(input, remember) => run(() => signIn(input, remember))} onDisconnect={() => void run(async () => {await invoke('disconnect_device');setSession(null);setSelected(null);})} />}
      </div> : selected ? <form className="desktop-card" onSubmit={e => {e.preventDefault(); void run(async () => {const value = sshSecret; const key = sshKey;setSshSecret('');setSshKey(null);setSession(await invoke<SessionInfo>('connect_device', {profileId:selected.id, sshSecret:sshKeyMode ? '' : value, sshKey:sshKeyMode ? key : null, sshKeyPassphrase:sshKeyMode ? value : null}));});}}>
        <h2>{selected.alias}</h2><p>{selected.connection.kind === 'ssh' ? selected.connection.host : selected.connection.origin}</p>
        {selected.connection.kind === 'ssh' && <>
          <label className="desktop-check"><input type="checkbox" checked={sshKeyMode} onChange={e => {setSshKeyMode(e.target.checked);setSshSecret('');setSshKey(null);}} />{copy("使用 SSH 私钥文件")}</label>
          {sshKeyMode && <label>{copy("SSH 私钥")}<input type="file" required onChange={async e => {const file=e.target.files?.[0];if(file && file.size <= 32768) setSshKey(await file.text()); else setError(copy("请选择不超过 32 KB 的 SSH 私钥文件。"));}} /><small>{copy("仅用于本次 SSH 登录，不保存在连接资料中。")}</small></label>}
          <label>{sshKeyMode ? copy("私钥口令（没有可留空）") : copy("SSH 密码")}<input type="password" required={!sshKeyMode} autoComplete="off" value={sshSecret} onChange={e => setSshSecret(e.target.value)} /></label><small>{copy("用于建立加密隧道。之后还需登录设备账户。")}</small></>}
        <div className="desktop-actions"><button type="button" disabled={busy} onClick={() => {setSelected(null);clearSsh();setError('');}}>{copy("返回")}</button><button className="primary" disabled={busy}>{busy ? copy("正在连接…") : copy("连接")}</button></div>
      </form> : <>
        <HomeAccounts busy={busy} onOpen={id => void run(async () => { await selectAccount(id); setAssetPage('assets'); })} />
        <div className="desktop-list-head"><h2>{copy("设备列表")} <span>{profiles.length}</span></h2><div className="wallet-actions"><button className="primary" disabled={busy} onClick={() => {setAdding(true);setError('');}}>{copy("＋ 添加设备")}</button></div></div>
        {profiles.length === 0 ? <div className="desktop-card desktop-empty"><div className="desktop-empty-icon">⌘</div><h2>{copy("连接第一台设备")}</h2><p>{copy("可以自动查找本机服务，也可以添加局域网设备的 HTTPS 或 SSH 地址。")}</p><button className="primary" onClick={() => setAdding(true)}>{copy("添加设备")}</button></div> : <div className="desktop-device-list">{profiles.map(profile => <article className="desktop-card device-row" key={profile.id}><div><h2>{profile.alias}</h2><p>{profile.connection.kind === 'ssh' ? profile.connection.host : profile.connection.origin}</p><span className="desktop-badge" role="status">{connectionLabel(profile.connection.kind)} · {connectingId === profile.id ? copy("正在连接…") : copy("待连接")}</span>{error && failedProfileId === profile.id && <p className="desktop-error" role="alert" ref={errorElement}>{copy(error)}</p>}</div><div className="desktop-actions"><button className="subtle" disabled={busy} onClick={() => {
          if (!window.confirm(copy(`忘记“${profile.alias}”并删除本机登录资料？设备上的账户、任务和数据会保留。`, `Forget “${profile.alias}” and remove its local sign-in details? Accounts, tasks and data on the device will remain.`))) return;
          void run(async () => {await invoke('forget_profile', {profileId:profile.id});forgetDeviceStorage(localStorage, profile.id);forgetDeviceStorage(sessionStorage, profile.id);await refresh();});
        }}>{copy("忘记")}</button><button disabled={busy} onClick={() => chooseProfile(profile)}>{connectingId === profile.id ? copy("正在连接…") : copy("连接")}</button></div></article>)}</div>}
      </>}
      {error && !failedProfileId && <p className="desktop-error" role="alert" ref={errorElement}>{copy(error)}</p>}
      <footer>{copy("本机可用 HTTP，局域网设备使用 HTTPS 或 SSH。连接资料只保存在这台电脑上。")}</footer>
    </main></div>;
}

void invoke<SessionInfo | null>('current_session').then(info => {
  if (info?.identity) return openConsole(info);
  root.render(<DeviceHome />);
}).catch(error => root.render(<p role="alert">{friendlyError(error)}</p>));
