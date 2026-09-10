import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { friendlyError } from './errors';
import { connectionLabel } from './connection-label';

export interface Candidate {
  name: string; address: string; kind: 'https' | 'ssh' | 'local';
  source: 'mdns' | 'subnet' | 'local'; private_ca: boolean; verified: false;
}
interface DiscoveryReport {
  local: {checked: boolean; installation_hint: boolean; running: boolean};
  candidates: Candidate[]; mdns_available: boolean; subnet_available: boolean;
  scanned_hosts: number; limited: boolean; cancelled: boolean;
}

export function DiscoveryPanel({ onSelect }: { onSelect: (candidate: Candidate) => void }) {
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [report, setReport] = useState<DiscoveryReport | null>(null);
  const [error, setError] = useState('');
  const mounted = useRef(false);
  const running = useRef(false);
  const search = useCallback(async (scanSubnet: boolean) => {
    if (running.current) return;
    running.current = true;
    setBusy(true); setScanning(scanSubnet); setError('');
    try {
      const result = await invoke<DiscoveryReport>('discover_devices', { scanSubnet });
      if (mounted.current) setReport(result);
    } catch (error) {
      if (mounted.current) setError(friendlyError(error));
    } finally {
      running.current = false;
      if (mounted.current) setBusy(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    void search(false);
    return () => { mounted.current = false; void invoke('cancel_discovery').catch(() => {}); };
  }, [search]);
  return <section className="desktop-discovery" aria-label="查找本机与局域网设备" aria-busy={busy}>
    <div className="desktop-discovery-head"><strong>本机与附近的设备</strong>
      {busy ? <button type="button" onClick={() => { void invoke('cancel_discovery').catch(e => setError(friendlyError(e))); }}>停止查找</button>
        : <button type="button" onClick={() => { void search(true); }}>扫描局域网</button>}
    </div>
    <p>自动检查本机程序、本地服务和同一网络中的设备。查找时不会发送登录信息。</p>
    <div role="status" aria-live="polite">
      {busy && <p>{scanning ? '正在扫描当前网段，通常需要 5–20 秒…' : '正在查找设备广播，约 4 秒…'}</p>}
      {!busy && report?.cancelled && <p>已停止查找，可以重新扫描或手工填写地址。</p>}
      {!busy && report && report.candidates.length === 0 && !report.cancelled && <p>暂未找到设备。请确认设备已开机、电脑与设备在同一局域网，再点击“扫描局域网”；也可以手工填写地址。</p>}
      {!busy && scanning && report && !report.subnet_available && <p>当前没有可扫描的有线或 Wi-Fi 内网连接，请手工填写地址。</p>}
      {!busy && report?.limited && <p>已完成有限范围查找。较大网段或其他 VLAN 的设备请手工添加。</p>}
      {!busy && report?.local.checked && <p className="desktop-local-status">{report.local.running ? '已发现本机运行中的服务，可通过本机 HTTP 连接。' : report.local.installation_hint ? '发现本机程序，但未检测到运行中的服务。请先启动服务；如使用自定义端口，可手工添加本机地址。' : '未检测到本机程序或运行中的服务。自定义安装位置或端口可能无法自动识别，可手工填写本机地址。'}</p>}
    </div>
    {report?.candidates.map(candidate => <button className="desktop-discovered" type="button" key={`${candidate.kind}:${candidate.address}`} onClick={() => onSelect(candidate)}>
      <span><strong>{candidate.source === 'local' ? '本机 · 当前电脑' : candidate.name}</strong><small>{candidate.address}</small></span>
      <span>{connectionLabel(candidate.kind)} · {candidate.kind === 'local' ? '可连接' : '待验证'}</span>
    </button>)}
    {!!report?.candidates.length && <small>本机 HTTP 只连接电脑内部地址，仍需登录。其他设备需验证证书或 SSH 主机指纹。</small>}
    {error && <p role="alert" className="desktop-error">{error}</p>}
  </section>;
}
