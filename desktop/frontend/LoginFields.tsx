import { copy } from "./i18n";
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { friendlyError } from './errors';
import type { LoginInput, LoginPrefill } from './types';

// The displayed mask is never submitted. An untouched prefill asks native login
// to load this session's profile credential directly from the OS credential store.
const SAVED_SECRET_MASK = '••••••••';

export function LoginFields({sessionId, busy, onLogin, onDisconnect}: {
  sessionId: string;
  busy: boolean;
  onLogin: (input: LoginInput | null, remember: boolean) => Promise<void>;
  onDisconnect: () => void;
}) {
  const [mode, setMode] = useState<LoginInput['mode']>('password');
  const [username, setUsername] = useState('');
  const [secret, setSecret] = useState('');
  const [saved, setSaved] = useState(false);
  const [remember, setRemember] = useState(false);
  const [loading, setLoading] = useState(true);
  const [prefillError, setPrefillError] = useState('');
  useEffect(() => {
    let active = true;
    setLoading(true);
    setMode('password'); setUsername(''); setSecret(''); setSaved(false); setRemember(false); setPrefillError('');
    void invoke<LoginPrefill | null>('login_prefill', {sessionId}).then(input => {
      if (!active || !input) return;
      setMode(input.mode); setUsername(input.username); setSaved(true); setRemember(true);
    }).catch(error => {
      if (active) setPrefillError(friendlyError(error));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => {active = false;};
  }, [sessionId]);
  const changeMode = (next: LoginInput['mode']) => {
    if (next === mode) return;
    setMode(next); setSaved(false); setSecret('');
  };
  const disabled = busy || loading;
  return <form className="desktop-login-form" onSubmit={event => {
    event.preventDefault();
    if (disabled) return;
    const input = saved ? null : {mode, username, secret};
    setSecret('');
    void onLogin(input, saved ? false : remember);
  }}>
    <div className="desktop-tabs">
      <button type="button" disabled={disabled} className={mode === 'password' ? 'selected' : ''} onClick={() => changeMode('password')}>{copy("用户名与密码")}</button>
      <button type="button" disabled={disabled} className={mode === 'key' ? 'selected' : ''} onClick={() => changeMode('key')}>{copy("用户 Key")}</button>
    </div>
    {mode === 'password' && <label>{copy("设备账户")}<input required disabled={disabled} autoComplete="username" value={username} onChange={event => {
      if (event.target.value !== username) {setSaved(false); setSecret('');}
      setUsername(event.target.value);
    }} /></label>}
    <label>{mode === 'key' ? copy("用户 Key") : copy("密码")}<input required disabled={disabled} type="password" autoComplete="off" value={saved ? SAVED_SECRET_MASK : secret} onFocus={event => {if (saved) event.currentTarget.select();}} onPaste={event => {
      if (!saved) return;
      event.preventDefault(); setSaved(false); setSecret(event.clipboardData.getData('text'));
    }} onChange={event => {
      const value = event.target.value;
      // Some native WebViews move the caret after focusing. The first edit
      // replaces the native-backed mask instead of appending it to a password.
      const inserted = (event.nativeEvent as InputEvent).data;
      setSecret(saved ? inserted ?? (value.startsWith(SAVED_SECRET_MASK) ? value.slice(SAVED_SECRET_MASK.length) : /^•*$/.test(value) ? '' : value) : value);
      setSaved(false);
    }} /></label>
    {saved && <small>{copy("已填入保存的登录信息，点击登录即可。")}</small>}
    {prefillError && <p className="desktop-note" role="status">{prefillError}</p>}
    <label className="desktop-check"><input type="checkbox" disabled={disabled} checked={remember} onChange={event => setRemember(event.target.checked)} />{copy("保存到系统凭据库")}</label>
    <small>{saved ? copy("修改登录信息后，可保存并替换原有记录。") : copy("不勾选时仅当前会话使用。设备账户权限仍由设备管理。")}</small>
    <div className="desktop-actions">
      <button type="button" disabled={disabled} onClick={onDisconnect}>{copy("断开")}</button>
      <button className="primary" disabled={disabled}>{loading ? copy("正在填入…") : busy ? copy("正在登录…") : copy("登录")}</button>
    </div>
  </form>;
}
