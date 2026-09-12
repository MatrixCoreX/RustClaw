import { connectionLabel } from './connection-label';
import { Channel, invoke } from '@tauri-apps/api/core';
import type { AuthIdentityResponse } from '../../UI/src/types/api';
import { scopedStorage } from './storage';
import { createTransport } from './transport';
import type { SessionInfo } from './types';

export let desktopIdentity: AuthIdentityResponse | null = null;
export let desktopOrigin = '';
export let desktopStorage: Storage;
export let desktopSessionStorage: Storage;
let session: SessionInfo;
let assetConnection: { id: string; node: { id: string; origin: string } } | null = null;
export let desktopFetch: (input: string, init?: RequestInit) => Promise<Response>;

export function initializeRuntime(info: SessionInfo) {
  if (session) throw new Error('desktop_runtime_already_initialized');
  session = info;
  desktopIdentity = info.identity;
  desktopOrigin = info.origin;
  const subject = `${info.identity?.user_id}:${info.identity?.chat_id}:${info.identity?.role}`;
  const scope = `agent-runtime.desktop.${info.profile.id}.${encodeURIComponent(subject)}.`;
  desktopStorage = scopedStorage(window.localStorage, scope);
  desktopSessionStorage = scopedStorage(window.sessionStorage, scope);
  desktopFetch = createTransport(invoke, info.id, info.origin, desktopLogout);
}

export function initializeStandaloneRuntime(info: { id: string; node: { id: string; origin: string } }) {
  assetConnection = info;
  desktopOrigin = info.node.origin;
  desktopStorage = scopedStorage(window.localStorage, `agent-runtime.desktop.asset-owner.${info.node.id}.`);
  desktopSessionStorage = scopedStorage(window.sessionStorage, `agent-runtime.desktop.asset-owner.${info.node.id}.`);
}
export function standaloneActive() { return assetConnection !== null; }
export function clearStandaloneRuntime() { assetConnection = null; }

export async function desktopLogout() {
  await invoke('disconnect_device');
  // Full realm replacement also destroys old hooks, signatures, drafts and delayed callbacks.
  window.location.reload();
}
export async function desktopMediaUrl(path: string) { return invoke<string>('media_open', {sessionId:session.id, path}); }
export async function desktopCloseMedia(url: string) { await invoke('media_close', {url}); }
export async function desktopDownload(path: string, filename: string) {
  const id = crypto.randomUUID();
  const progress = new Channel<{id: string; written: number; total: number | null; finished: boolean}>();
  const update = (detail: object) => window.dispatchEvent(new CustomEvent('desktop-download', {detail: {id, filename, ...detail}}));
  progress.onmessage = data => update(data);
  update({written:0, total:null, finished:false});
  try {
    const saved = await invoke<boolean>('download', {sessionId: session.id, path, filename, id, progress});
    update({finished:true, cancelled:!saved});
    return saved;
  } catch (error) { update({finished:true, error:String(error)}); throw error; }
}
export async function desktopCancelDownload(id: string) { await invoke('download_cancel', {sessionId:session.id, id}); }
export async function desktopOpenAipp(skillName: string, locale: string) {
  return invoke('aipp_open', {sessionId: session.id, skillName, locale});
}
export function desktopSigningLocation() {
  // Only the packaged renderer with a native authenticated session reaches this path.
  // Local HTTP is restricted to literal loopback addresses by the native transport.
  return session?.identity ? {protocol: 'https:', hostname: 'desktop.localhost'} : {protocol: 'blocked:', hostname: ''};
}
export function desktopTargetLabel() {
  if (assetConnection) return `资产服务 · ${assetConnection.node.origin}`;
  return session ? `${session.profile.alias} · ${connectionLabel(session.profile.connection.kind)} · ${session.profile.connection.kind === 'ssh' ? session.profile.connection.host : session.origin}` : '';
}
export function desktopSessionId() {
  const id = assetConnection?.id ?? session?.id;
  if (!id) throw new Error('wallet_node_missing');
  return id;
}
