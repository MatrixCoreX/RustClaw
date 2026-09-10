import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import './aipp.css';

interface Scope {session_id: string; skill_name: string; package_version: string; entrypoint: string; bridge_capabilities: string[]; locale: string}
void (async () => {
  const scope = await invoke<Scope>('aipp_context');
  const frame = document.createElement('iframe');
  frame.sandbox.add('allow-scripts');
  frame.referrerPolicy = 'no-referrer';
  const origin = new URL(convertFileSrc('', 'device'));
  frame.src = `${origin.protocol}//${origin.host}/${scope.session_id}/v1/aipps/${encodeURIComponent(scope.skill_name)}/assets/${scope.entrypoint.split('/').map(encodeURIComponent).join('/')}`;
  frame.title = scope.skill_name;
  const context = () => frame.contentWindow?.postMessage({schema_version:1, type:'aipp.host.context', locale:scope.locale, skill_name:scope.skill_name, package_version:scope.package_version, capabilities:scope.bridge_capabilities}, '*');
  frame.onload = context;
  const pending = new Set<string>();
  window.addEventListener('message', e => {
    if (e.source !== frame.contentWindow || !e.data || typeof e.data !== 'object') return;
    const request = e.data;
    if (request.schema_version !== 1) return;
    if (request.type === 'aipp.ready') {context(); return;}
    if (request.type !== 'aipp.capability.invoke' || typeof request.request_id !== 'string' || request.request_id.length > 128 || typeof request.capability !== 'string') return;
    const reply = (ok: boolean, data?: unknown, error_code?: string) => frame.contentWindow?.postMessage({schema_version:1, type:'aipp.capability.result', request_id:request.request_id, ok, data, error_code}, '*');
    if (!scope.bridge_capabilities.includes(request.capability) || pending.size >= 4 || pending.has(request.request_id)) {reply(false, undefined, 'aipp_bridge_denied'); return;}
    pending.add(request.request_id);
    void invoke('aipp_bridge', {capability:request.capability, args:request.args ?? {}}).then(data => reply(true, data)).catch(() => reply(false, undefined, 'aipp_bridge_failed')).finally(() => pending.delete(request.request_id));
  });
  document.getElementById('root')!.appendChild(frame);
})().catch(() => {document.getElementById('root')!.textContent = '应用不可用，请关闭窗口并从设备控制台重新打开。';});
