import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { desktopSessionId } from "../runtime";
import { walletError } from "./errors";
import type {
  Capabilities,
  Intent,
  OperationRecord,
  ReadResult,
  Service,
  WalletAccount,
} from "./types";

export function useAssetAccount(
  account: WalletAccount,
  service: Service,
  nodeHint: string,
  enabled = true,
) {
  const scope = `${enabled}:${account.id}:${service}:${nodeHint}`;
  const current = useRef(scope);
  current.current = scope;
  const [data, setData] = useState<ReadResult | null>(null);
  const [cap, setCap] = useState<Capabilities | null>(null);
  const [records, setRecords] = useState<OperationRecord[]>([]);
  const previousRecords = useRef<Map<string, string> | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const base = {
    sessionId: desktopSessionId(),
    accountId: account.id,
    service,
  };
  const refresh = useCallback(
    async (page?: number) => {
      if (!enabled) return;
      const expected = scope;
      setBusy(true);
      setError("");
      try {
        const cap = await invoke<Capabilities>("wallet_capabilities", {
          sessionId: desktopSessionId(),
          service,
        });
        if (current.current !== expected) return;
        setCap(cap);
        const data = await invoke<ReadResult>("wallet_read", {
          sessionId: desktopSessionId(),
          accountId: account.id,
          service,
          page: page ?? null,
        });
        if (current.current === expected) setData(data);
      } catch (e) {
        if (current.current === expected) {
          setData(null);
          setError(walletError(e));
        }
      } finally {
        if (current.current === expected) setBusy(false);
      }
    },
    [scope],
  );
  useEffect(() => {
    current.current = scope;
    setData(null);
    setCap(null);
    setError("");
    setMessage("");
    void refresh();
    return () => {
      current.current = "unmounted";
    };
  }, [refresh]);
  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const records = await invoke<OperationRecord[]>(
          "wallet_operations",
          base,
        );
        if (active) {
          const previous = previousRecords.current;
          previousRecords.current = new Map(
            records.map((r) => [r.operation_id, r.status]),
          );
          setRecords(records);
          if (
            previous &&
            records.some(
              (r) =>
                r.status === "succeeded" &&
                previous.get(r.operation_id) !== "succeeded",
            )
          ) {
            void refresh();
          }
        }
      } catch {
        if (active) setRecords([]);
      }
    };
    void load();
    const timer = setInterval(() => void load(), 1500);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [account.id, service, nodeHint, refresh]);
  const prepare = async (intent: Intent) => {
    if (busy) return;
    const expected = scope;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const operationId = await invoke<string>("wallet_prepare", {
        ...base,
        intent,
      });
      if (current.current === expected)
        setMessage("已打开本地安全窗口，请核对实际交易内容并确认。");
      return current.current === expected ? operationId : null;
    } catch (e) {
      if (current.current === expected) setError(walletError(e));
    } finally {
      if (current.current === expected) setBusy(false);
    }
  };
  const check = async (operationId: string) => {
    const expected = scope;
    setBusy(true);
    setError("");
    try {
      await invoke("wallet_check_operation", { ...base, operationId });
      if (current.current === expected) {
        setMessage("已核实操作状态。");
        await refresh();
      }
    } catch (e) {
      if (current.current === expected) setError(walletError(e));
    } finally {
      if (current.current === expected) setBusy(false);
    }
  };
  return {
    data: enabled ? data : null,
    cap: enabled ? cap : null,
    records,
    busy,
    error,
    message,
    refresh,
    prepare,
    check,
    clearFeedback: () => {
      setError("");
      setMessage("");
    },
    ready:
      enabled &&
      account.backed_up &&
      Boolean(data) &&
      Boolean(cap) &&
      !records.some((r) => r.status === "pending"),
  };
}
