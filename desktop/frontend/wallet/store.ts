import { useEffect, useState, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WalletStatus } from "./types";
import { walletError } from "./errors";

interface Snapshot extends WalletStatus {
  selectedId: string;
  error: string;
}
let snapshot: Snapshot = {
  initialized: false,
  unlocked: false,
  accounts: [],
  selectedId: "",
  error: "",
};
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | undefined;
let refreshing = false;
let selectionSequence = 0;
function update(next: Snapshot) {
  if (JSON.stringify(next) !== JSON.stringify(snapshot)) {
    snapshot = next;
    listeners.forEach((fn) => fn());
  }
}
export async function refreshWallet() {
  if (refreshing) return;
  refreshing = true;
  try {
    const status = await invoke<WalletStatus>("wallet_status");
    update({ ...snapshot, ...status, error: "" });
  } catch (error) {
    update({ ...snapshot, unlocked: false, error: walletError(error) });
  } finally {
    refreshing = false;
  }
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (!timer) {
    void refreshWallet();
    timer = setInterval(() => void refreshWallet(), 1000);
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size && timer) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}
export function useWallet() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
export async function selectAccount(id: string) {
  const sequence = ++selectionSequence;
  await invoke("wallet_select", { accountId: id || null });
  if (sequence === selectionSequence) update({ ...snapshot, selectedId: id });
}
export function useAccountContext(context: string) {
  const wallet = useWallet();
  const key = `${context}:${wallet.selectedId}`;
  const [readyFor, setReadyFor] = useState("");
  useEffect(() => {
    let active = true;
    void selectAccount(wallet.selectedId)
      .then(() => {
        if (active) setReadyFor(key);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [key]);
  return { ...wallet, contextReady: readyFor === key };
}
export function openWallet() {
  return invoke("wallet_open");
}
export async function lockWallet() {
  await invoke("wallet_lock");
  await refreshWallet();
}
