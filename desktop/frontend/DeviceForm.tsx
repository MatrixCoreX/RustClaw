import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Connection, Profile } from './types';
import { friendlyError } from './errors';
import { DiscoveryPanel } from './DiscoveryPanel';

export function DeviceForm({onAdded, onCancel}: {onAdded: (p: Profile) => void; onCancel: () => void}) {
  const [kind, setKind] = useState<'https' | 'ssh'>('https');
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
        : {kind, host: address.trim(), port, username: username.trim(), host_key_sha256: fingerprint.trim(), webd_port: webdPort};
      if ((privateCa || kind === 'ssh') && !confirmed) throw new Error('certificate_fingerprint_required');
      onAdded(await invoke<Profile>('add_profile', {alias, connection}));
    } catch (e) { setError(friendlyError(e)); } finally { setBusy(false); }
  }}>
    <h2>添加设备</h2><p>填写已安装服务的设备地址。连接成功后，再登录设备账户。</p>
    <DiscoveryPanel onSelect={candidate => {
      setKind(candidate.kind); setAddress(candidate.address);
      setAlias(current => current || candidate.name);
      setPort(22); setWebdPort(8788);
      setPrivateCa(candidate.private_ca); setPem(''); setFingerprint(''); setConfirmed(false); setError('');
    }} />
    <label>设备名称<input required autoFocus placeholder="例如：书房设备" maxLength={128} value={alias} onChange={e => setAlias(e.target.value)} /></label>
    <div className="desktop-tabs"><button type="button" className={kind === 'https' ? 'selected' : ''} onClick={() => {setKind('https'); setConfirmed(false); setFingerprint('');}}>HTTPS · 推荐</button><button type="button" className={kind === 'ssh' ? 'selected' : ''} onClick={() => {setKind('ssh'); setConfirmed(false); setFingerprint('');}}>SSH 加密隧道</button></div>
    <label>{kind === 'https' ? 'HTTPS 地址' : 'SSH 主机地址'}<input required placeholder={kind === 'https' ? 'https://device.local:8443' : 'device.local 或 192.168.1.20'} value={address} onChange={e => {setAddress(e.target.value); setConfirmed(false);}} /></label>
    {kind === 'https' ? <>
      <label className="desktop-check"><input type="checkbox" checked={privateCa} onChange={e => setPrivateCa(e.target.checked)} />设备使用私有 CA / 自签名证书</label>
      {privateCa && <label>公开 CA 证书<input type="file" accept=".crt,.pem,.cer" onChange={async e => {const file = e.target.files?.[0]; if (file) {if (file.size > 32768) {setError('证书文件过大。'); return;} setPem(await file.text()); setConfirmed(false);}}} /><small>从设备本地、已可信控制台或已核验 SSH 获取，不要上传任何私钥。</small></label>}
    </> : <div className="desktop-columns">
      <label>SSH 用户名<input required value={username} onChange={e => setUsername(e.target.value)} autoComplete="off" /></label>
      <label>SSH 端口<input type="number" min={1} max={65535} value={port} onChange={e => setPort(Number(e.target.value))} /></label>
      <label>设备 WEBD 端口<input type="number" min={1} max={65535} value={webdPort} onChange={e => setWebdPort(Number(e.target.value))} /></label>
    </div>}
    {(privateCa || kind === 'ssh') && <>
      <label>{kind === 'ssh' ? 'SSH 主机公钥指纹' : 'CA 证书 SHA-256 指纹'}<input required placeholder={kind === 'ssh' ? 'SHA256:…' : '完整的 64 位十六进制指纹'} value={fingerprint} onChange={e => {setFingerprint(e.target.value); setConfirmed(false);}} spellCheck={false} /></label>
      <p className="desktop-note">请在设备本地屏幕或已可信入口核对完整指纹。局域网发现结果和同一条未验证网络连接不能证明设备身份。</p>
      <label className="desktop-check"><input type="checkbox" required checked={confirmed} onChange={e => setConfirmed(e.target.checked)} />我已通过可信入口核对设备指纹</label>
    </>}
    {error && <p role="alert" className="desktop-error">{error}</p>}
    <div className="desktop-actions"><button type="button" onClick={onCancel} disabled={busy}>返回</button><button className="primary" disabled={busy}>{busy ? '正在保存…' : '保存设备'}</button></div>
  </form>;
}
