import { useCallback, useRef, useState } from "react";

import type { ApiResponse, NniAssetTransferHistoryResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

interface AssetTransferHistoryFailureData {
  attempts?: Array<{ error_code?: string }>;
}

export function useAssetTransferHistoryRuntime({
  apiFetch,
  t,
}: {
  apiFetch: ApiFetch;
  t: Translate;
}) {
  const apiFetchRef = useRef(apiFetch);
  const translateRef = useRef(t);
  const requestSequence = useRef(0);
  const [history, setHistory] = useState<NniAssetTransferHistoryResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  apiFetchRef.current = apiFetch;
  translateRef.current = t;

  const load = useCallback(async (ownerPublicKey: string) => {
    const normalizedOwner = ownerPublicKey.trim();
    const sequence = ++requestSequence.current;
    if (!normalizedOwner) {
      setHistory(null);
      setLoading(false);
      setError(null);
      return null;
    }
    setHistory(null);
    setLoading(true);
    setError(null);
    try {
      const response = await apiFetchRef.current(
        `/v1/nni/assets/transfers?owner_pubkey=${encodeURIComponent(normalizedOwner)}&limit=10`,
      );
      const body = (await response.json()) as ApiResponse<
        NniAssetTransferHistoryResponse | AssetTransferHistoryFailureData
      >;
      if (
        !response.ok
        || !body.ok
        || !body.data
        || !("transactions" in body.data)
        || body.data.owner_pubkey !== normalizedOwner
      ) {
        throw new Error(body.error ?? "asset_transfer_history_unavailable");
      }
      const result = body.data as NniAssetTransferHistoryResponse;
      if (sequence === requestSequence.current) setHistory(result);
      return result;
    } catch {
      if (sequence === requestSequence.current) {
        setError(translateRef.current(
          "转账历史暂时无法读取，请稍后重试。",
          "Transfer history is temporarily unavailable. Try again later.",
        ));
      }
      return null;
    } finally {
      if (sequence === requestSequence.current) setLoading(false);
    }
  }, []);

  return { history, loading, error, load };
}
