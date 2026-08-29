export const ASSET_HISTORY_REMOTE_BATCH_SIZE = 100;
export const ASSET_HISTORY_DISPLAY_PAGE_SIZE = 10;
export const ASSET_HISTORY_DISPLAY_PAGES_PER_BATCH =
  ASSET_HISTORY_REMOTE_BATCH_SIZE / ASSET_HISTORY_DISPLAY_PAGE_SIZE;

export type AssetHistorySourceFilter = "all" | "transfer" | "trade" | "issuance";
export type AssetHistoryDirectionFilter = "all" | "incoming" | "outgoing";

export interface AssetHistoryLoadOptions {
  source?: AssetHistorySourceFilter;
  direction?: AssetHistoryDirectionFilter;
  displayPage?: number;
  force?: boolean;
}

export function normalizeAssetHistoryDisplayPage(displayPage: number): number {
  return Number.isSafeInteger(displayPage) && displayPage > 0 ? displayPage : 1;
}

export function assetHistoryRemotePage(displayPage: number): number {
  const safePage = normalizeAssetHistoryDisplayPage(displayPage);
  return Math.floor((safePage - 1) / ASSET_HISTORY_DISPLAY_PAGES_PER_BATCH) + 1;
}

export function assetHistoryLocalTransactionOffset(displayPage: number): number {
  const safePage = normalizeAssetHistoryDisplayPage(displayPage);
  return ((safePage - 1) % ASSET_HISTORY_DISPLAY_PAGES_PER_BATCH)
    * ASSET_HISTORY_DISPLAY_PAGE_SIZE;
}

export function assetHistoryDisplayTotalPages(totalTransactions: number): number {
  if (!Number.isSafeInteger(totalTransactions) || totalTransactions <= 0) return 0;
  return Math.ceil(totalTransactions / ASSET_HISTORY_DISPLAY_PAGE_SIZE);
}

export function assetHistoryRequestPath(
  ownerPublicKey: string,
  source: AssetHistorySourceFilter,
  direction: AssetHistoryDirectionFilter,
  displayPage: number,
): string {
  const params = new URLSearchParams({
    owner_pubkey: ownerPublicKey,
    limit: String(ASSET_HISTORY_REMOTE_BATCH_SIZE),
    page: String(assetHistoryRemotePage(displayPage)),
    source,
    direction,
  });
  return `/v1/nni/assets/transfers?${params.toString()}`;
}
