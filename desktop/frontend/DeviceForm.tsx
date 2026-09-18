import { copy } from "./i18n";
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Connection, Profile } from './types';
import { friendlyError } from './errors';
import { DiscoveryPanel } from './DiscoveryPanel';

export function DeviceForm({onAdded, onCancel}: {onAdded: (p: Profile) => void; onCancel: () => void}) {
  const [kind, setKind] = useState<Connection['kind']>('https');
  const [alias, setAlias] = useState('');
  const [address, setAddress] = useState('');
  const [privateCa, setPrivateCa] = useState(false);
  const [pem, setPem] = useState('');
  const [fingerprint, setFingerprint] = useState('');
  const [username, setUsername] = useState('');
  const [port, setPort] = useState(22);
  const [webdPort, setWebdPort] = useState(8788);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  return <form className="desktop-card device-form" onSubmit={async e => {
    e.preventDefault(); setBusy(true); setError('');
    try {
      const connection: Connection = kind === 'https'
        ? {kind, origin: address.trim(), ca_pem: privateCa ? pem : null, ca_sha256: privateCa ? fingerprint.trim() : null}
        : kind === 'local' ? {kind, origin: address.trim()}
        : {kind, host: address.trim(), port, username: username.trim(), host_key_sha256: fingerprint.trim(), webd_port: webdPort};
      if (((kind === 'https' && privateCa) || kind === 'ssh') && !confirmed) throw new Error('certificate_fingerprint_required');
      onAdded(await invoke<Profile>('add_profile', {alias, connection}));
    } catch (e) { setError(friendlyError(e)); } finally { setBusy(false); }
  }}>
    <h2>{copy("添加设备")}</h2><p>{copy("填写已安装服务的设备地址。连接成功后，再登录设备账户。")}</p>
    <DiscoveryPanel onSelect={candidate => {
      setKind(candidate.kind); setAddress(candidate.address);
      setAlias(current => current || (candidate.kind === 'local' ? copy("本机") : candidate.name));
      setPort(22); setWebdPort(8788);
      setPrivateCa(candidate.private_ca); setPem(''); setFingerprint(''); setConfirmed(false); setError('');
    }} />
    <label>{copy("设备名称")}<input required autoFocus placeholder={copy("例如：书房设备")} maxLength={128} value={alias} onChange={e => setAlias(e.target.value)} /></label>
    <div className="desktop-tabs"><button type="button" className={kind === 'https' ? 'selected' : ''} onClick={() => {setKind('https'); setAddress(''); setConfirmed(false); setFingerprint('');}}>{copy("HTTPS · 局域网")}</button><button type="button" className={kind === 'ssh' ? 'selected' : ''} onClick={() => {setKind('ssh'); setAddress(''); setConfirmed(false); setFingerprint('');}}>{copy("SSH 加密隧道")}</button><button type="button" className={kind === 'local' ? 'selected' : ''} onClick={() => {setKind('local'); setAddress('http://127.0.0.1:8788'); setPrivateCa(false); setAlias(current => current || copy("本机"));}}>{copy("本机 HTTP")}</button></div>
    <label>{kind === 'https' ? copy("HTTPS 地址") : kind === 'local' ? copy("本机服务地址") : copy("SSH 主机地址")}<input required placeholder={kind === 'https' ? 'https://device.local:8443' : kind === 'local' ? 'http://127.0.0.1:8788' : copy("device.local 或 192.168.1.20")} value={address} onChange={e => {setAddress(e.target.value); setConfirmed(false);}} /></label>
    {kind === 'https' ? <>
      <label className="desktop-check"><input type="checkbox" checked={privateCa} onChange={e => setPrivateCa(e.target.checked)} />{copy("设备使用私有 CA / 自签名证书")}</label>
      {privateCa && <label>{copy("公开 CA 证书")}<input type="file" accept=".crt,.pem,.cer" onChange={async e => {const file = e.target.files?.[0]; if (file) {if (file.size > 32768) {setError(copy("证书文件过大。")); return;} setPem(await file.text()); setConfirmed(false);}}} /><small>{copy("从设备本地、已可信控制台或已核验 SSH 获取，不要上传任何私钥。")}</small></label>}
    </> : kind === 'local' ? <p className="desktop-note">{copy("仅连接这台电脑上的服务，地址只允许 127.0.0.1 或 ::1。HTTP 不加密，但流量不会发往局域网；连接后仍需登录账户。")}</p> : <div className="desktop-columns">
      <label>{copy("SSH 用户名")}<input required value={username} onChange={e => setUsername(e.target.value)} autoComplete="off" /></label>
      <label>{copy("SSH 端口")}<input type="number" min={1} max={65535} value={port} onChange={e => setPort(Number(e.target.value))} /></label>
      <label>{copy("设备 WEBD 端口")}<input type="number" min={1} max={65535} value={webdPort} onChange={e => setWebdPort(Number(e.target.value))} /></label>
    </div>}
    {((kind === 'https' && privateCa) || kind === 'ssh') && <>
      <label>{kind === 'ssh' ? copy("SSH 主机公钥指纹") : copy("CA 证书 SHA-256 指纹")}<input required placeholder={kind === 'ssh' ? 'SHA256:…' : copy("完整的 64 位十六进制指纹")} value={fingerprint} onChange={e => {setFingerprint(e.target.value); setConfirmed(false);}} spellCheck={false} /></label>
      <p className="desktop-note">{copy("请在设备本地屏幕或已可信入口核对完整指纹。局域网发现结果和同一条未验证网络连接不能证明设备身份。")}</p>
      <label className="desktop-check"><input type="checkbox" required checked={confirmed} onChange={e => setConfirmed(e.target.checked)} />{copy("我已通过可信入口核对设备指纹")}</label>
    </>}
    {error && <p role="alert" className="desktop-error">{copy(error)}</p>}
    <div className="desktop-actions"><button type="button" onClick={onCancel} disabled={busy}>{copy("返回")}</button><button className="primary" disabled={busy}>{busy ? copy("正在保存…") : kind === 'ssh' ? copy("下一步") : copy("保存并连接")}</button></div>
  </form>;
}
