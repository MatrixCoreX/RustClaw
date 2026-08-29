import { useCallback, useRef, useState } from "react";

import {
  ASSET_HISTORY_REMOTE_BATCH_SIZE,
  assetHistoryRemotePage,
  assetHistoryRequestPath,
  type AssetHistoryDirectionFilter,
  type AssetHistoryLoadOptions,
  type AssetHistorySourceFilter,
} from "../lib/asset-transfer-history";
import type { ApiResponse, NniAssetTransferHistoryResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

interface AssetTransferHistoryFailureData {
  attempts?: Array<{ error_code?: string }>;
}

const ASSET_HISTORY_CACHE_LIMIT = 24;

function assetHistoryCacheKey(
  ownerPublicKey: string,
  source: AssetHistorySourceFilter,
  direction: AssetHistoryDirectionFilter,
  displayPage: number,
): string {
  return [ownerPublicKey, source, direction, assetHistoryRemotePage(displayPage)].join("\n");
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
  const cacheRef = useRef(new Map<string, NniAssetTransferHistoryResponse>());
  const [history, setHistory] = useState<NniAssetTransferHistoryResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  apiFetchRef.current = apiFetch;
  translateRef.current = t;

  const load = useCallback(async (
    ownerPublicKey: string,
    options: AssetHistoryLoadOptions = {},
  ) => {
    const normalizedOwner = ownerPublicKey.trim();
    const source = options.source ?? "all";
    const direction = options.direction ?? "all";
    const displayPage = options.displayPage ?? 1;
    const remotePage = assetHistoryRemotePage(displayPage);
    const cacheKey = assetHistoryCacheKey(normalizedOwner, source, direction, displayPage);
    const sequence = ++requestSequence.current;
    if (!normalizedOwner) {
      setHistory(null);
      setLoading(false);
      setError(null);
      return null;
    }
    const cached = cacheRef.current.get(cacheKey);
    if (cached && !options.force) {
      setHistory(cached);
      setLoading(false);
      setError(null);
      return cached;
    }
    if (!cached) setHistory(null);
    setLoading(true);
    setError(null);
    try {
      const response = await apiFetchRef.current(
        assetHistoryRequestPath(normalizedOwner, source, direction, displayPage),
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
        || body.data.page !== remotePage
        || body.data.per_page !== ASSET_HISTORY_REMOTE_BATCH_SIZE
        || body.data.source_filter !== source
        || body.data.direction_filter !== direction
      ) {
        throw new Error(body.error ?? "asset_transfer_history_unavailable");
      }
      const result = body.data as NniAssetTransferHistoryResponse;
      cacheRef.current.delete(cacheKey);
      cacheRef.current.set(cacheKey, result);
      while (cacheRef.current.size > ASSET_HISTORY_CACHE_LIMIT) {
        const oldestKey = cacheRef.current.keys().next().value;
        if (typeof oldestKey !== "string") break;
        cacheRef.current.delete(oldestKey);
      }
      if (sequence === requestSequence.current) setHistory(result);
      return result;
    } catch {
      if (sequence === requestSequence.current) {
        setError(translateRef.current(
          "资产流水暂时无法读取，请稍后重试。",
          "Asset activity is temporarily unavailable. Try again later.",
        ));
      }
      return null;
    } finally {
      if (sequence === requestSequence.current) setLoading(false);
    }
  }, []);

  return { history, loading, error, load };
}
